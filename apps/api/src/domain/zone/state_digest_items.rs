//! `BinaryV6` canonical mutable item layout; immutable rule content is bound by its hash.
use super::Writer;
use crate::domain::zone::phase4::{
    ItemActionBinding, ItemPayment, Phase4Player, Phase4Snapshot, ShieldOutcome,
};
use crate::domain::zone::phase4_checkpoint::{EquipmentSourceKey, ItemReuseKey, SetSkillKind};
use crate::domain::zone::{CastInFlight, ItemInstanceId, ItemLocation};

pub(super) fn append(w: &mut Writer, p: &Phase4Snapshot) {
    w.u32(p.formula_revision);
    w.u32(p.timer_policy_version);
    w.0.extend_from_slice(&p.rules_hash);
    let (start, next, end) = p.allocator.bounds();
    w.u64(start);
    w.u64(next);
    w.u64(end);
    w.len(p.players.len());
    for (id, player) in &p.players {
        w.id(*id);
        w.item_player(player);
    }
    w.len(p.facing.len());
    for (id, facing) in &p.facing {
        w.id(*id);
        let (x, y) = facing.components();
        w.i64(x);
        w.i64(y);
    }
    w.len(p.item_casts.len());
    for (id, context) in &p.item_casts {
        w.u64(id.0);
        w.binding(&context.caster);
        w.item_cast(&context.cast);
        w.option(context.origin.as_ref(), |w, origin| {
            w.item_id(&origin.instance);
            w.u32(origin.template);
            w.u64(origin.count);
            w.u32(origin.skill_ordinal);
            w.0.extend_from_slice(origin.receipt.key.as_bytes());
        });
        w.bool(context.simultaneous);
        w.option(context.weapon.as_ref(), Writer::item_id);
        w.u8(context.shots);
    }
    w.len(p.attack_plans.len());
    for (id, plan) in &p.attack_plans {
        w.id(*id);
        w.binding(&plan.attacker);
        w.u64(plan.start.0);
        w.u64(plan.ready.0);
        w.option(plan.weapon.as_ref(), Writer::item_id);
        w.u8(plan.shots);
        w.option(plan.arrow.as_ref(), Writer::item_payment);
        w.u32(plan.next_hit);
        w.len(plan.hits.len());
        for hit in &plan.hits {
            w.binding(&hit.target);
            w.u64(hit.at.0);
            w.option(hit.prepared.as_ref(), |w, result| {
                use crate::domain::zone::AttackOutcome;
                w.u8(match result.outcome {
                    AttackOutcome::Miss => 0,
                    AttackOutcome::Hit => 1,
                    AttackOutcome::Crit => 2,
                });
                w.u8(match result.shield {
                    ShieldOutcome::None => 0,
                    ShieldOutcome::Normal => 1,
                    ShieldOutcome::Perfect => 2,
                });
                w.u32(result.damage);
            });
        }
    }
    w.len(p.charge_bindings.len());
    for binding in &p.charge_bindings {
        w.item_id(&binding.weapon);
        w.u8(binding.shot.bit());
        match binding.action {
            ItemActionBinding::Cast { id } => {
                w.u8(0);
                w.u64(id.0);
            },
            ItemActionBinding::Attack { attacker, start } => {
                w.u8(1);
                w.id(attacker);
                w.u64(start.0);
            },
        }
    }
}
impl Writer {
    #[allow(clippy::trivially_copy_pass_by_ref)] // Shared optional writer callback signature.
    fn item_id(&mut self, id: &ItemInstanceId) {
        self.u64(id.get());
    }
    fn item_payment(&mut self, p: &ItemPayment) {
        self.item_id(&p.instance);
        self.u32(p.template);
        self.u64(p.count);
    }
    fn item_player(&mut self, p: &Phase4Player) {
        self.u64(p.inventory_revision);
        self.len(p.inventory.len());
        for (key, item) in &p.inventory {
            self.item_id(key);
            self.item_id(&item.id);
            self.u32(item.template_id);
            self.u64(item.count);
            self.u8(match item.location {
                ItemLocation::Inventory => 0,
                ItemLocation::Paperdoll => 1,
            });
            self.option(item.slot.as_ref(), |w, slot| w.u8(*slot as u8));
            self.u32(item.enchant);
            self.option(item.augment_id.as_ref(), |w, id| w.u32(*id));
            self.u8(item.charges);
            self.u64(item.created_tick.0);
            self.u64(item.revision);
        }
        self.len(p.paperdoll.len());
        for (slot, id) in &p.paperdoll {
            self.u8(*slot as u8);
            self.item_id(id);
        }
        self.len(p.auto_shots.len());
        for id in &p.auto_shots {
            self.u32(*id);
        }
        self.len(p.item_reuse.len());
        for reuse in &p.item_reuse {
            match reuse.key {
                ItemReuseKey::Template(id) => {
                    self.u8(0);
                    self.u32(id);
                },
                ItemReuseKey::SharedGroup(id) => {
                    self.u8(1);
                    self.u32(id);
                },
            }
            self.u64(reuse.until.0);
        }
        self.len(p.equipment_sources.len());
        for (sequence, source) in &p.equipment_sources {
            self.u64(*sequence);
            self.item_source(source);
        }
        self.len(p.active_sets.len());
        for (id, set) in &p.active_sets {
            self.u32(*id);
            self.item_id(&set.chest);
            self.bool(set.full_set_active);
            self.bool(set.shield_skill_active);
            self.bool(set.enchant6_active);
        }
        self.i64(p.weight);
        self.i64(p.max_load);
        self.u32(p.ordinary_capacity);
        self.u32(p.quest_capacity);
        self.i64(p.weight_penalty_bonus);
        self.u8(p.overload_level);
        self.bool(p.overloaded);
        self.u8(p.expertise);
        self.u8(p.weapon_penalty);
        self.u8(p.armor_penalty);
        let receipt = &p.starter_kit_receipt;
        self.u32(receipt.manifest_version);
        self.0.extend_from_slice(&receipt.manifest_hash);
        self.u32(receipt.source_class.0);
        self.0.extend_from_slice(receipt.request_id.as_bytes());
        self.u64(receipt.materialized_revision);
    }
    fn item_source(&mut self, source: &EquipmentSourceKey) {
        match source {
            EquipmentSourceKey::Item { instance } => {
                self.u8(0);
                self.item_id(instance);
            },
            EquipmentSourceKey::ItemSkill {
                instance,
                xml_ordinal,
            } => {
                self.u8(1);
                self.item_id(instance);
                self.u32(*xml_ordinal);
            },
            EquipmentSourceKey::Enchant4 { instance } => {
                self.u8(2);
                self.item_id(instance);
            },
            EquipmentSourceKey::EnchantOption { instance, option } => {
                self.u8(3);
                self.item_id(instance);
                self.u32(*option);
            },
            EquipmentSourceKey::Augment { instance, half } => {
                self.u8(4);
                self.item_id(instance);
                self.u8(*half);
            },
            EquipmentSourceKey::ArmorSet {
                chest,
                set,
                skill_kind,
                ordinal,
            } => {
                self.u8(5);
                self.item_id(chest);
                self.u32(*set);
                self.u8(match skill_kind {
                    SetSkillKind::Full => 0,
                    SetSkillKind::Shield => 1,
                    SetSkillKind::Enchant6 => 2,
                });
                self.u32(*ordinal);
            },
            EquipmentSourceKey::GradePenalty { weapon } => {
                self.u8(6);
                self.bool(*weapon);
            },
            EquipmentSourceKey::Overload => self.u8(7),
            EquipmentSourceKey::Polearm => self.u8(8),
        }
    }
    fn item_cast(&mut self, c: &CastInFlight) {
        self.u64(c.id.0);
        self.u32(c.caster_incarnation);
        self.skill_ref(c.skill);
        self.u64(c.command_ordinal.0);
        self.u64(c.start_at.0);
        self.ratio(c.animation);
        self.u64(c.launch_at.0);
        self.u64(c.complete_at.0);
        self.u64(c.interrupt_until.0);
        self.bool(c.launched);
        self.option(c.target.as_ref(), Writer::binding);
        self.u32(c.affect_limit);
        self.len(c.targets.len());
        for target in &c.targets {
            self.binding(target);
        }
        self.u32(c.mp_start);
        self.u32(c.mp_end);
        self.u32(c.hp_end);
        self.u64(c.reuse_until.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_retained_capacities_change_the_binary_layout() {
        let mut player = crate::domain::zone::item_functions::tests::player();
        let encode = |player: &Phase4Player| {
            let mut writer = Writer(Vec::new());
            writer.item_player(player);
            writer.0
        };
        let original = encode(&player);
        player.ordinary_capacity += 1;
        assert_ne!(encode(&player), original);
        player.ordinary_capacity -= 1;
        player.quest_capacity += 1;
        assert_ne!(encode(&player), original);
    }
}
