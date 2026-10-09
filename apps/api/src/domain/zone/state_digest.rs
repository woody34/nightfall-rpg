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

impl ZoneState {
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
            CombatRole::Player { class, xp } => {
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
