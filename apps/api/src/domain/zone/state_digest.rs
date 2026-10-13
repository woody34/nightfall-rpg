//! Binary v3 binds a cached SHA-256 of the complete resolved class registry, then the
//! unchanged `BinaryV2` digest and ordered integer player identity/ledger/receipt fields.
//! Registry hashing runs only on bootstrap and validated restore.
//!
//! Binary v2 digest layout. SHA-256 over `nightfall.state.2\0`, then tick, ordinal,
//! RNG (32-byte key, stream, word position), entities, hate ledgers and spawn members.
//! Integers use their declared fixed width, little endian; UUIDs use their 16 RFC bytes.
//! Sequences/UTF-8 strings have u64 byte/item counts, options and bools have a one-byte
//! 0/1 tag. Enum tags are explicit below. Struct fields follow the writer order, with no
//! padding or serde omissions. Maps use key order. Changing this layout needs a new version.

use super::{
    CombatRole, CombatState, Entity, EntityId, EntityKind, RngState, Vec2Fixed, ZoneState,
};
use crate::domain::zone::{Intention, MemberState, NpcAi, SlotMember, Swing, TargetingState};
use sha2::{Digest as _, Sha256};

#[path = "state_digest_items.rs"]
mod items;

impl ZoneState {
    pub(super) fn phase4_state_digest(&self) -> [u8; 32] {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(b"nightfall-state-v6\0");
        w.0.extend_from_slice(&self.phase3_state_digest());
        w.option(self.phase4.as_ref(), |w, p| items::append(w, &p.canonical));
        Sha256::digest(&w.0).into()
    }

    pub(super) fn phase3_state_digest(&self) -> [u8; 32] {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(b"nightfall-state-v5\0");
        w.0.extend_from_slice(&self.token_state_digest());
        w.option(self.meta.phase3_admission_policy.as_ref(), |w, policy| {
            w.u32(policy.version);
            w.u64(policy.record_budget);
            w.u64(policy.max_command_bytes);
            w.u32(policy.max_observers);
        });
        w.option(self.phase3.as_ref(), |w, p| {
            let p = &p.canonical;
            w.0.extend_from_slice(&p.rules_hash);
            w.u64(p.next_entity_ordinal);
            w.u64(p.next_cast_id);
            w.u64(p.next_effect_id);
            w.u64(p.next_offer_id);
            w.u64(p.next_install_sequence);
            w.len(p.entities.len());
            for (id, entity) in &p.entities {
                w.id(*id);
                w.phase3_entity(entity);
            }
        });
        Sha256::digest(&w.0).into()
    }

    pub(super) fn token_state_digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"nightfall.state.4\0token-policy.1\0");
        hash.update(self.phase2_state_digest());
        hash.finalize().into()
    }

    pub(super) fn phase2_state_digest(&self) -> [u8; 32] {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(b"nightfall.state.3\0");
        w.option(self.meta.classes_rules_hash.as_ref(), |w, hash| w.0.extend_from_slice(hash));
        // The immutable legacy layout is incorporated as a fixed-width component.
        w.0.extend_from_slice(&self.binary_state_digest());
        w.len(self.entities.len());
        for e in self.entities.values() {
            w.id(e.id);
            let p = match e.combat.as_ref().map(|c| &c.role) {
                Some(CombatRole::Player { progression, .. }) => progression.as_deref(),
                _ => None,
            };
            w.option(p, Writer::progression);
        }
        Sha256::digest(&w.0).into()
    }

    pub(super) fn binary_state_digest(&self) -> [u8; 32] {
        // One contiguous buffer avoids tiny hash updates and all per-entity allocations.
        let mut w = Writer(Vec::with_capacity(self.entities.len().saturating_mul(320)));
        w.0.extend_from_slice(b"nightfall.state.2\0");
        w.u64(self.next_tick.0);
        w.u64(self.next_ordinal.0);
        let RngState {
            key,
            stream,
            word_pos,
        } = RngState::capture(&self.rng);
        w.0.extend_from_slice(&key);
        w.u64(stream);
        w.0.extend_from_slice(&word_pos.to_le_bytes());
        w.len(self.entities.len());
        for entity in self.entities.values() {
            w.entity(entity);
        }
        w.len(self.hate.len());
        for (id, ledger) in &self.hate {
            w.id(*id);
            w.len(ledger.iter().count());
            for (attacker, entry) in ledger.iter() {
                w.id(attacker);
                w.u64(entry.hate);
                w.u64(entry.damage);
            }
        }
        w.len(self.members.len());
        for MemberState {
            member,
            entity,
            incarnation,
            respawn_at,
        } in self.members.values()
        {
            w.slot(*member);
            w.option(entity.as_ref(), |w, id| w.id(*id));
            w.u32(*incarnation);
            w.option(respawn_at.as_ref(), |w, tick| w.u64(tick.0));
        }
        Sha256::digest(&w.0).into()
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn binding(&mut self, value: &crate::domain::zone::effects::TargetBinding) {
        self.id(value.entity);
        self.u32(value.incarnation);
    }

    fn skill_ref(&mut self, value: crate::domain::zone::skill::SkillRef) {
        self.u32(value.id.0);
        self.u32(value.rank);
    }

    fn ratio(&mut self, value: crate::domain::zone::effects::FrozenRatio) {
        let (numerator, denominator) = value.value().parts();
        self.0.extend_from_slice(&numerator.to_le_bytes());
        self.0.extend_from_slice(&denominator.to_le_bytes());
    }

    fn phase3_entity(&mut self, value: &crate::domain::zone::Phase3Entity) {
        use crate::domain::zone::effects::{
            EffectPool, EffectSourceKind, PeriodicPower, PeriodicResource,
        };
        self.u64(value.ordinal.0);
        match &value.cast {
            crate::domain::zone::CastState::Idle => self.u8(0),
            crate::domain::zone::CastState::Casting(cast) => {
                self.u8(1);
                self.u64(cast.id.0);
                self.u32(cast.caster_incarnation);
                self.skill_ref(cast.skill);
                self.u64(cast.command_ordinal.0);
                self.u64(cast.start_at.0);
                self.ratio(cast.animation);
                self.u64(cast.launch_at.0);
                self.u64(cast.complete_at.0);
                self.u64(cast.interrupt_until.0);
                self.u8(u8::from(cast.launched));
                self.option(cast.target.as_ref(), Self::binding);
                self.u32(cast.affect_limit);
                self.len(cast.targets.len());
                for target in &cast.targets {
                    self.binding(target);
                }
                self.u32(cast.mp_start);
                self.u32(cast.mp_end);
                self.u32(cast.hp_end);
                self.u64(cast.reuse_until.0);
            },
        }
        self.len(value.reuse.len());
        for (id, at) in &value.reuse {
            self.u32(id.0);
            self.u64(at.0);
        }
        self.len(value.effects.entries().len());
        for (id, effect) in value.effects.entries() {
            self.u64(id.0);
            self.u64(effect.id.0);
            self.skill_ref(effect.skill);
            self.u8(match effect.source.kind {
                EffectSourceKind::Player => 0,
                EffectSourceKind::Npc => 1,
                EffectSourceKind::System => 2,
            });
            self.option(effect.source.binding.as_ref(), Self::binding);
            self.u32(effect.target_incarnation);
            self.u64(effect.applied_at.0);
            self.option(effect.expires_at.as_ref(), |w, at| w.u64(at.0));
            self.u8(match effect.pool {
                EffectPool::Buff => 0,
                EffectPool::SongDance => 1,
                EffectPool::Debuff => 2,
                EffectPool::System => 3,
            });
            self.string(&effect.abnormal_type);
            self.u32(effect.abnormal_level);
            for flag in [
                effect.flags.stay_after_death,
                effect.flags.stay_on_subclass_change,
                effect.flags.removed_on_damage,
                effect.flags.removed_on_action_except_move,
                effect.flags.irreplaceable,
            ] {
                self.u8(u8::from(flag));
            }
            self.len(effect.handlers.len());
            for handler in &effect.handlers {
                self.u32(handler.xml_ordinal);
                self.option(handler.periodic.as_ref(), |w, periodic| {
                    w.u8(match periodic.resource {
                        PeriodicResource::Hp => 0,
                        PeriodicResource::Mp => 1,
                    });
                    match periodic.power {
                        PeriodicPower::Diff { amount } => {
                            w.u8(0);
                            w.ratio(amount);
                        },
                        PeriodicPower::Per { power, multiplier } => {
                            w.u8(1);
                            w.ratio(power);
                            w.ratio(multiplier);
                        },
                    }
                    w.u64(periodic.schedule.anchor.0);
                    w.u64(periodic.schedule.next_ideal_offset_ms);
                    w.u64(periodic.schedule.pulse_index);
                    w.u32(periodic.schedule.interval_ms);
                });
            }
        }
        self.len(value.modifier_install_order.len());
        for (sequence, source) in &value.modifier_install_order {
            self.u64(*sequence);
            match source {
                crate::domain::zone::RestoredModifierSource::Passive(skill) => {
                    self.u8(0);
                    self.skill_ref(*skill);
                },
                crate::domain::zone::RestoredModifierSource::Effect(id) => {
                    self.u8(1);
                    self.u64(id.0);
                },
                crate::domain::zone::RestoredModifierSource::DeathPenalty => self.u8(2),
            }
        }
        self.option(value.regen.next_at.as_ref(), |w, at| w.u64(at.0));
        self.i64(value.fractions.hp);
        self.i64(value.fractions.mp);
        self.i64(value.fractions.cp);
        self.option(value.player.as_ref(), Self::phase3_player);
    }

    fn phase3_player(&mut self, player: &crate::domain::zone::Phase3Player) {
        self.option(player.pvp.flag_until.as_ref(), |w, at| w.u64(at.0));
        self.u32(player.pvp.karma);
        self.u32(player.pvp.pk_count);
        self.u32(player.pvp.pvp_count);
        self.u8(player.death_penalty_level);
        self.option(player.last_death.as_ref(), |w, death| {
            w.u32(death.incarnation);
            w.u64(death.pre_death_xp);
            w.u64(death.recoverable_death_xp_loss);
        });
        self.option(player.resurrect_offer.as_ref(), |w, offer| {
            w.u64(offer.id.0);
            w.binding(&offer.caster);
            w.u32(offer.target_incarnation);
            w.skill_ref(offer.skill);
            w.u64(offer.created_at.0);
            w.u64(offer.expires_at.0);
            w.i64(offer.restore_percent_q);
            w.u64(offer.recoverable_xp);
        });
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn identity(&mut self, i: &crate::domain::character_progression::CharacterIdentity) {
        self.0.extend_from_slice(i.account_id.as_uuid().as_bytes());
        self.string(i.race.as_str());
        self.u32(i.base_class_id.0);
        self.u32(match i.appearance.sex {
            crate::domain::subclass::Sex::Male => 1,
            crate::domain::subclass::Sex::Female => 2,
        });
        self.u32(i.appearance.hair_style);
        self.u32(i.appearance.hair_color);
        self.u32(i.appearance.face);
    }
    fn progression(&mut self, p: &crate::domain::zone::PlayerProgression) {
        self.identity(&p.identity);
        let c = &p.class_state;
        self.u32(c.base_class_id.0);
        self.u32(c.current_class_id.0);
        self.u64(c.sp);
        self.u32(c.cp);
        self.u32(p.max_cp);
        self.u32(c.token_tier_1_count);
        self.u32(c.token_tier_2_count);
        self.0.push(c.milestone_claimed_mask);
        self.len(c.learned_skills.len());
        for skill in &c.learned_skills {
            self.string(&skill.key);
            self.u32(skill.level);
        }
        self.len(c.successful_transfer_receipts.len());
        for receipt in &c.successful_transfer_receipts {
            self.0.extend_from_slice(receipt.key.as_bytes());
            self.u32(receipt.target_class_id.0);
            let r = &receipt.result;
            self.0
                .extend_from_slice(r.character_id.as_uuid().as_bytes());
            self.identity(&r.identity);
            self.string(r.name.as_str());
            self.u32(r.current_class_id.0);
            self.u32(r.level);
            self.u64(r.xp);
            self.u64(r.sp);
            for n in [
                r.stats.str,
                r.stats.dex,
                r.stats.con,
                r.stats.int,
                r.stats.wit,
                r.stats.men,
            ] {
                self.u32(n);
            }
            for n in r.position_millitiles {
                self.0.extend_from_slice(&n.to_le_bytes());
            }
            for n in [
                r.hp,
                r.mp,
                r.cp,
                r.max_hp,
                r.max_mp,
                r.max_cp,
                r.token_tier_1_count,
                r.token_tier_2_count,
            ] {
                self.u32(n);
            }
            self.len(r.granted_skill_keys.len());
            for key in &r.granted_skill_keys {
                self.string(key);
            }
        }
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn len(&mut self, value: usize) {
        // Supported targets have at most 64-bit pointers.
        self.u64(value as u64);
    }

    fn bool(&mut self, value: bool) {
        self.0.push(u8::from(value));
    }

    fn id(&mut self, id: EntityId) {
        self.0.extend_from_slice(id.as_uuid().as_bytes());
    }

    fn pos(&mut self, pos: Vec2Fixed) {
        self.0.extend_from_slice(&pos.x.raw().to_le_bytes());
        self.0.extend_from_slice(&pos.y.raw().to_le_bytes());
    }

    fn string(&mut self, value: &str) {
        self.len(value.len());
        self.0.extend_from_slice(value.as_bytes());
    }

    fn option<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) {
        self.bool(value.is_some());
        if let Some(value) = value {
            write(self, value);
        }
    }

    fn slot(&mut self, SlotMember { slot, member }: SlotMember) {
        self.0.extend_from_slice(&slot.to_le_bytes());
        self.0.extend_from_slice(&member.to_le_bytes());
    }

    fn entity(&mut self, entity: &Entity) {
        // Exhaustive destructuring makes new state fields a compile error here.
        let Entity {
            id,
            kind,
            name,
            pos,
            dest,
            speed,
            generation,
            targeting,
            combat,
            ai,
        } = entity;
        self.id(*id);
        self.0.push(match kind {
            EntityKind::Player => 0,
            EntityKind::Npc => 1,
        });
        self.string(name);
        self.pos(*pos);
        self.option(dest.as_ref(), |w, p| w.pos(*p));
        self.u32(speed.milli_tiles_per_tick());
        self.u64(generation.0);
        let TargetingState {
            target,
            dead,
            attackable,
        } = targeting;
        self.option(target.as_ref(), |w, id| w.id(*id));
        self.bool(*dead);
        self.bool(*attackable);
        self.option(combat.as_ref(), Self::combat);
        self.option(ai.as_ref(), Self::ai);
    }

    fn combat(&mut self, combat: &CombatState) {
        let CombatState {
            role,
            sheet,
            hp,
            mp,
            attack_range,
            collision_radius,
            incarnation,
            auto_attack,
            chasing,
            swing,
            ready_at,
            protected_until,
        } = combat;
        match role {
            CombatRole::Player {
                class,
                xp,
                progression: _,
            } => {
                self.0.push(0);
                self.string(class);
                self.u64(*xp);
            },
            CombatRole::Npc {
                template,
                xp_reward,
            } => {
                self.0.push(1);
                self.string(template);
                self.u64(*xp_reward);
            },
        }
        let crate::domain::zone::FinalStats {
            level,
            max_hp,
            max_mp,
            p_atk,
            p_def,
            accuracy,
            evasion,
            crit_permille,
            attack_speed,
            random_damage,
        } = (*sheet).into();
        self.u32(level);
        self.u32(max_hp);
        self.u32(max_mp);
        self.i64(p_atk.raw());
        self.i64(p_def.raw());
        self.i64(accuracy.raw());
        self.i64(evasion.raw());
        self.u32(crit_permille);
        self.i64(attack_speed.raw());
        self.u32(random_damage);
        self.u32(*hp);
        self.u32(*mp);
        self.0.extend_from_slice(&attack_range.raw().to_le_bytes());
        self.0
            .extend_from_slice(&collision_radius.raw().to_le_bytes());
        self.u32(*incarnation);
        self.bool(*auto_attack);
        self.bool(*chasing);
        self.option(
            swing.as_ref(),
            |w,
             Swing {
                 target,
                 target_incarnation,
                 start,
                 impact,
                 ready,
             }| {
                w.id(*target);
                w.u32(*target_incarnation);
                w.u64(start.0);
                w.u64(impact.0);
                w.u64(ready.0);
            },
        );
        self.u64(ready_at.0);
        self.option(protected_until.as_ref(), |w, tick| w.u64(tick.0));
    }

    fn ai(&mut self, ai: &NpcAi) {
        let NpcAi {
            slot,
            home,
            intention,
            last_hit,
            called_help,
            corpse_until,
        } = ai;
        self.slot(*slot);
        self.pos(*home);
        self.0.push(match intention {
            Intention::Idle => 0,
            Intention::Active => 1,
            Intention::Attack => 2,
            Intention::ReturnHome => 3,
            Intention::Dead => 4,
        });
        self.u64(last_hit.0);
        self.bool(*called_help);
        self.option(corpse_until.as_ref(), |w, tick| w.u64(tick.0));
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod phase3_digest_tests {
    use super::*;
    use crate::domain::zone::effects::*;
    use crate::domain::zone::phase3_entity::tests::{blank, cast, source_effect};
    use crate::domain::zone::skill::{SkillId, SkillRef};
    use crate::domain::zone::*;
    use serde_json::Value;

    fn bytes(entity: &Phase3Entity) -> Vec<u8> {
        let mut writer = Writer(Vec::new());
        writer.phase3_entity(entity);
        writer.0
    }

    fn specimen() -> Phase3Entity {
        let rules = crate::infrastructure::skill_data::load_phase3_rules(
            &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
            &[],
        )
        .unwrap();
        let mut entity = blank();
        let binding = cast().target.unwrap();
        entity.cast = CastState::Casting(cast());
        entity.reuse.insert(SkillId(1177), Tick(100));
        let mut effect = source_effect(&rules, 1, 110);
        effect.expires_at = Some(Tick(100));
        effect.source = EffectSource {
            binding: Some(binding),
            kind: EffectSourceKind::Player,
        };
        effect.handlers = vec![
            ActiveHandler {
                xml_ordinal: 0,
                periodic: Some(ActivePeriodic {
                    resource: PeriodicResource::Hp,
                    power: PeriodicPower::Diff {
                        amount: FrozenRatio::new(skill_math::ExactRatio::whole(1)),
                    },
                    schedule: PulseSchedule::new(Tick(0), 666).unwrap(),
                }),
            },
            ActiveHandler {
                xml_ordinal: 2,
                periodic: Some(ActivePeriodic {
                    resource: PeriodicResource::Mp,
                    power: PeriodicPower::Per {
                        power: FrozenRatio::new(skill_math::ExactRatio::whole(1)),
                        multiplier: FrozenRatio::new(skill_math::ExactRatio::whole(1)),
                    },
                    schedule: PulseSchedule::new(Tick(0), 666).unwrap(),
                }),
            },
        ];
        entity.effects =
            EffectList::from_entries(std::collections::BTreeMap::from([(effect.id, effect)]))
                .unwrap();
        entity.modifier_install_order = std::collections::BTreeMap::from([
            (
                1,
                RestoredModifierSource::Passive(SkillRef {
                    id: SkillId(1),
                    rank: 1,
                }),
            ),
            (2, RestoredModifierSource::Effect(EffectId(1))),
            (3, RestoredModifierSource::DeathPenalty),
        ]);
        let player = entity.player.as_mut().unwrap();
        player.pvp.flag_until = Some(Tick(100));
        player.death_penalty_level = 1;
        player.last_death = Some(DeathRecord {
            incarnation: 1,
            pre_death_xp: 100,
            recoverable_death_xp_loss: 10,
        });
        player.resurrect_offer = Some(ResurrectOffer {
            id: OfferId(1),
            caster: binding,
            target_incarnation: 1,
            skill: SkillRef {
                id: SkillId(1016),
                rank: 1,
            },
            created_at: Tick(0),
            expires_at: Tick(600),
            restore_percent_q: 30_000_000,
            recoverable_xp: 10,
        });
        entity
    }

    // This is a serialization-coverage probe, not a legal gameplay fixture (it deliberately
    // includes both a cast and a corpse offer to exercise every independent writer branch).
    #[test]
    fn every_canonical_scalar_is_in_the_binary_entity_layout() {
        fn leaves(node: &Value, path: &str, out: &mut Vec<(String, Value)>) {
            match node {
                Value::Object(map) => {
                    for (key, value) in map {
                        leaves(value, &format!("{path}/{key}"), out);
                    }
                },
                Value::Array(array) => {
                    for (i, value) in array.iter().enumerate() {
                        leaves(value, &format!("{path}/{i}"), out);
                    }
                },
                Value::Number(n) => {
                    if path == "/effects/entries/1/id" {
                        return;
                    }
                    let increment = if path.ends_with("/interval_ms") {
                        666
                    } else {
                        1
                    };
                    out.push((path.into(), Value::from(n.as_u64().unwrap() + increment)));
                },
                Value::Bool(b) => out.push((path.into(), Value::Bool(!b))),
                Value::String(s) if s.contains('/') => {
                    out.push((path.into(), Value::String("7/3".into())));
                },
                Value::String(s) if s.contains('-') => {
                    out.push((path.into(), Value::String(uuid::Uuid::from_u128(999).to_string())));
                },
                Value::String(_) if path.ends_with("/abnormal_type") => {
                    out.push((path.into(), Value::String("PROBE".into())));
                },
                Value::Null | Value::String(_) => {},
            }
        }
        let original = specimen();
        let encoded = bytes(&original);
        let json = serde_json::to_value(&original).unwrap();
        let mut mutations = Vec::new();
        leaves(&json, "", &mut mutations);
        assert!(mutations.len() >= 70, "{} scalar probes", mutations.len());
        for (path, value) in mutations {
            let mut changed = json.clone();
            *changed.pointer_mut(&path).unwrap() = value;
            let changed: Phase3Entity = serde_json::from_value(changed).unwrap();
            assert_ne!(encoded, bytes(&changed), "omitted canonical field {path}");
        }
        for path in [
            "/cast",
            "/effects",
            "/reuse",
            "/modifier_install_order",
            "/player",
            "/regen/next_at",
        ] {
            let mut changed = json.clone();
            *changed.pointer_mut(path).unwrap() = match path {
                "/cast" => serde_json::json!({"state":"idle"}),
                "/effects" => serde_json::json!({"entries":{}}),
                "/reuse" | "/modifier_install_order" => serde_json::json!({}),
                _ => Value::Null,
            };
            let changed: Phase3Entity = serde_json::from_value(changed).unwrap();
            assert_ne!(encoded, bytes(&changed), "omitted presence/length {path}");
        }
        let restored: Phase3Entity = serde_json::from_value(json).unwrap();
        assert_eq!(encoded, bytes(&restored));
    }
}
