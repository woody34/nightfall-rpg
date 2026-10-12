//! V6 blow caller and shared source shield/facing getters.
use super::*;
use crate::domain::zone::item_combat::{self, BlowTerms, LethalOutcome, Proximity};
use crate::domain::zone::phase4::ShieldOutcome;

impl ZoneState {
    fn relative_position_v6(
        &self,
        caster: EntityId,
        target: EntityId,
    ) -> Result<(i64, i64), TickError> {
        let a = self.entities.get(&caster).ok_or_else(missing)?.pos;
        let b = self.entities.get(&target).ok_or_else(missing)?.pos;
        Ok((
            i64::from(a.x.raw())
                .checked_sub(i64::from(b.x.raw()))
                .ok_or_else(missing)?,
            i64::from(a.y.raw())
                .checked_sub(i64::from(b.y.raw()))
                .ok_or_else(missing)?,
        ))
    }

    pub(super) fn proximity_v6(
        &self,
        caster: EntityId,
        target: EntityId,
    ) -> Result<Proximity, TickError> {
        let Some(items) = self.phase4.as_ref() else {
            return Ok(Proximity::Front);
        };
        let facing = *items.canonical.facing.get(&target).ok_or_else(missing)?;
        let (dx, dy) = self.relative_position_v6(caster, target)?;
        Proximity::classify(facing, dx, dy).map_err(invariant)
    }

    pub(super) fn shield_v6(
        &mut self,
        caster: TargetBinding,
        target: TargetBinding,
        draw: &mut DamageRoll<'_>,
    ) -> Result<ShieldOutcome, TickError> {
        let Some(items) = self.phase4.as_ref() else {
            return Ok(ShieldOutcome::None);
        };
        let Some(player) = items.canonical.players.get(&target.entity) else {
            return Ok(ShieldOutcome::None);
        };
        let Some(shield) = player.paperdoll.get(&crate::domain::zone::EquipSlot::Lhand) else {
            return Ok(ShieldOutcome::None);
        };
        let item = player.inventory.get(shield).ok_or_else(missing)?;
        let definition = items
            .rules
            .parts()
            .items
            .get(&item.template_id)
            .ok_or_else(missing)?;
        if definition.kind != "Armor" || definition.armor_type == "SIGIL" {
            return Ok(ShieldOutcome::None);
        }
        let b = self.effective(target.entity)?;
        let (_, dex) = self.base_property_v5(target.entity, "DEX")?;
        let rate = self
            .damage_stat_v5(target.entity, &b, "rShld", 0)?
            .checked_mul(dex)
            .map_err(invariant)?;
        if !rate
            .checked_cmp(ExactRatio::new(1, i128::from(Q)).map_err(invariant)?)
            .map_err(invariant)?
            .is_gt()
        {
            return Ok(ShieldOutcome::None);
        }
        let angle = self
            .damage_stat_v5(target.entity, &b, "shieldDefAngle", 0)?
            .trunc()
            .map_err(invariant)?
            .checked_add(120)
            .ok_or_else(missing)?;
        if angle < 360 {
            if angle < 0 {
                return Ok(ShieldOutcome::None);
            }
            let certificate = items
                .rules
                .parts()
                .angles
                .get(usize::try_from(angle).map_err(invariant)?)
                .ok_or_else(missing)?;
            let facing = items
                .canonical
                .facing
                .get(&target.entity)
                .ok_or_else(missing)?;
            let (dx, dy) = self.relative_position_v6(caster.entity, target.entity)?;
            if !facing
                .within(dx, dy, certificate, items.rules.parts().coordinate_bound_millitiles)
                .map_err(invariant)?
            {
                return Ok(ShieldOutcome::None);
            }
        }
        let bow = self.item_weapon_kind(caster.entity, "NONE")? == "BOW";
        item_combat::shield(true, rate, bow, |site, bound| draw(self, site, target, bound))
            .map_err(invariant)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn blow_handler_v6(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        shots: u8,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        self.blow_handler_rolled_v6(
            tick,
            caster,
            target,
            skill,
            rank,
            handler,
            shots,
            events,
            &mut draw_damage_v5,
        )
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Source caller order differs from PhysicalAttack.
    pub(super) fn blow_handler_rolled_v6(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        shots: u8,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<SkillTargetResult, TickError> {
        if self.phase4.is_none() {
            return Err(missing());
        }
        let a = self.effective(caster.entity)?;
        let b = self.effective(target.entity)?;
        let mut result = SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Physical,
            outcome: SkillOutcome::Miss,
            rolled: 0,
            cp_absorbed: 0,
            hp_removed: 0,
            heal: 0,
            target_hp_after: self
                .entities
                .get(&target.entity)
                .and_then(|v| v.combat.as_ref())
                .ok_or_else(missing)?
                .hp,
            applied_effect_ids: Vec::new(),
        };
        let evasion = self.damage_stat_v5(target.entity, &b, "pSkillEvas", 0)?;
        if skill_math::physical_skill_checks(
            skill.is_magic == 1,
            skill.abnormal.as_ref().is_some_and(|v| v.is_debuff),
            evasion,
            0,
            ExactRatio::whole(1),
            |bound| draw(self, DamageRollSite::PhysicalEvasion, target, bound),
        )
        .map_err(invariant)?
        .is_none()
        {
            return Ok(result);
        }
        let parameter = |name: &str| {
            handler
                .params
                .get(name)
                .and_then(|v| v.at(rank, skill.max_level))
                .map_or(Ok(ExactRatio::whole(0)), |v| ExactRatio::parse_literal(v))
                .map_err(invariant)
        };
        let proximity = self.proximity_v6(caster.entity, target.entity)?;
        let (_, dex) = self.base_property_v5(caster.entity, "DEX")?;
        let rate = proximity
            .blow_rate(parameter("blowChance")?, dex)
            .map_err(invariant)?
            .to_q()
            .map_err(invariant)?;
        let rate = self.damage_stat_v5(caster.entity, &a, "blowRate", rate)?;
        let residue = draw(self, DamageRollSite::BlowSuccess, target, 1000).map_err(invariant)?;
        if !item_combat::blow_lands(rate, residue).map_err(invariant)?
            || self
                .entities
                .get(&caster.entity)
                .ok_or_else(missing)?
                .targeting
                .dead
        {
            return Ok(result);
        }
        let shield = self.shield_v6(caster, target, draw)?;
        let damage = if shield == ShieldOutcome::Perfect {
            ExactRatio::whole(1)
        } else {
            let defence = q(b.physical.p_def().raw())
                .map_err(invariant)?
                .checked_add(if shield == ShieldOutcome::Normal {
                    ExactRatio::whole(
                        i64::try_from(
                            self.damage_stat_v5(target.entity, &b, "sDef", 0)?
                                .trunc()
                                .map_err(invariant)?,
                        )
                        .map_err(invariant)?,
                    )
                } else {
                    ExactRatio::whole(0)
                })
                .map_err(invariant)?
                .checked_mul(self.item_pvp_damage_bonus(
                    target.entity,
                    caster.entity,
                    &b,
                    "pvpPhysSkillsDef",
                )?)
                .map_err(invariant)?;
            let base = BlowTerms {
                power: parameter("power")?,
                attack: q(a.physical.p_atk().raw()).map_err(invariant)?,
                defence,
                critical: self.damage_stat_v5(caster.entity, &a, "critDmg", Q)?,
                critical_position: self.damage_stat_v5(caster.entity, &a, "critDmgPos", Q)?,
                critical_vulnerability: self.damage_stat_v5(
                    target.entity,
                    &b,
                    "defCritDamage",
                    Q,
                )?,
                critical_add: self.damage_stat_v5(caster.entity, &a, "critDmgAdd", 0)?,
                critical_add_vulnerability: self.damage_stat_v5(
                    target.entity,
                    &b,
                    "defCritDamageAdd",
                    0,
                )?,
                proximity,
                soulshot: shots & crate::domain::zone::phase4::ShotKind::Soul.bit() != 0,
                pvp_bonus: self.item_pvp_damage_bonus(
                    caster.entity,
                    target.entity,
                    &a,
                    "pvpPhysSkillsDmg",
                )?,
            }
            .damage()
            .map_err(invariant)?;
            let weapon = self.item_weapon_kind(caster.entity, "NONE")?;
            let weapon_trait = ExactRatio::whole(2)
                .checked_sub(b.traits.get(weapon).cloned().unwrap_or_default().defence)
                .map_err(invariant)?;
            let damage = base
                .checked_mul(weapon_trait)
                .map_err(invariant)?
                .checked_mul(Self::trait_modifier_v5(&a, &b, &skill.r#trait)?)
                .map_err(invariant)?
                .checked_mul(skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?)
                .map_err(invariant)?
                .checked_mul(self.spread_v5(a.physical.random_damage(), target, draw)?)
                .map_err(invariant)?
                .checked_mul(self.npc_penalty_v5(
                    caster.entity,
                    target.entity,
                    crate::domain::zone::skill_npc::NpcPenaltyPath::Blow,
                )?)
                .map_err(invariant)?;
            if damage
                .checked_cmp(ExactRatio::whole(1))
                .map_err(invariant)?
                .is_lt()
            {
                ExactRatio::whole(1)
            } else {
                damage
            }
        };
        let chance = parameter("criticalChance")?;
        let critical = if chance
            .checked_cmp(ExactRatio::whole(0))
            .map_err(invariant)?
            .is_gt()
        {
            let (_, strength) = self.base_property_v5(caster.entity, "STR")?;
            let value = draw(self, DamageRollSite::PhysicalCritical, target, 100_000_000)
                .map_err(invariant)?;
            if value >= 100_000_000 {
                return Err(missing());
            }
            chance
                .checked_mul(strength)
                .map_err(invariant)?
                .checked_cmp(ExactRatio::new(i128::from(value), i128::from(Q)).map_err(invariant)?)
                .map_err(invariant)?
                .is_gt()
        } else {
            false
        };
        result.rolled = u32::try_from(
            damage
                .checked_mul(ExactRatio::whole(if critical { 2 } else { 1 }))
                .map_err(invariant)?
                .trunc()
                .map_err(invariant)?,
        )
        .map_err(invariant)?;
        let applied = self.apply_damage_rolled_v5(
            tick,
            DamageRequest {
                source: self.damage_source_v5(caster)?,
                target,
                rolled_damage: result.rolled,
                cause: DamageCause::Direct,
            },
            events,
            draw,
        )?;
        result.cp_absorbed = applied.cp_absorbed;
        result.hp_removed = applied.hp_removed;
        result.target_hp_after = applied.hp_after;
        result.outcome = if critical {
            SkillOutcome::Crit
        } else {
            SkillOutcome::Hit
        };
        self.attack_break_v5(tick, target, result.rolled, events, draw)?;
        // The admitted manifest has no vengeance provider; the source still draws at zero chance.
        if skill.is_magic != 1
            && *skill
                .cast_range
                .at(rank, skill.max_level)
                .ok_or_else(missing)?
                <= 40
        {
            draw(self, DamageRollSite::PhysicalVengeance, target, 100).map_err(invariant)?;
        }
        Ok(result)
    }
}

impl ZoneState {
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn lethal_handler_v6(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        _events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        self.lethal_handler_rolled_v6(
            tick,
            caster,
            target,
            skill,
            rank,
            handler,
            &mut draw_damage_v5,
        )
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Keep eligibility, conditional draws and direct setters together.
    pub(super) fn lethal_handler_rolled_v6(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        draw: &mut DamageRoll<'_>,
    ) -> Result<SkillTargetResult, TickError> {
        if self.phase4.is_none()
            || self.binding(caster.entity) != Some(caster)
            || self.binding(target.entity) != Some(target)
        {
            return Err(missing());
        }
        let victim = self.entities.get(&target.entity).ok_or_else(missing)?;
        let c = victim.combat.as_ref().ok_or_else(missing)?;
        let old_hp = c.hp;
        let old_fraction = self.sidecar(target.entity)?.fractions.hp;
        let player = victim.kind == EntityKind::Player;
        // Admitted NPC profiles have no raid, minion, grand-boss or door kind.
        // Spawn protection is the admitted invulnerability source.
        let eligible = !victim.targeting.dead
            && !c.protected_at(tick)
            && *skill
                .magic_level
                .get(usize::try_from(rank.checked_sub(1).ok_or_else(missing)?).map_err(invariant)?)
                .ok_or_else(missing)?
                >= i64::from(c.sheet.level()).saturating_sub(6);
        let parameter = |name: &str| {
            handler
                .params
                .get(name)
                .and_then(|v| v.at(rank, skill.max_level))
                .map_or(Ok(ExactRatio::whole(0)), |v| ExactRatio::parse_literal(v))
                .map_err(invariant)
        };
        let full = parameter("fullLethal")?;
        let half = parameter("halfLethal")?;
        let multiplier = if eligible {
            Self::trait_modifier_v5(
                self.effective(caster.entity)?.as_ref(),
                self.effective(target.entity)?.as_ref(),
                &skill.r#trait,
            )?
            .checked_mul(skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?)
            .map_err(invariant)?
        } else {
            ExactRatio::whole(1)
        };
        let outcome = item_combat::lethal(eligible, full, half, multiplier, |site, bound| {
            draw(self, site, target, bound)
        })
        .map_err(invariant)?;
        let next_hp = match outcome {
            LethalOutcome::Full => (1, 0),
            LethalOutcome::Half if !player => {
                item_combat::half_hp(old_hp, old_fraction).map_err(invariant)?
            },
            LethalOutcome::None | LethalOutcome::Half => (old_hp, old_fraction),
        };
        if outcome != LethalOutcome::None {
            let c = self
                .entities
                .get_mut(&target.entity)
                .and_then(|v| v.combat.as_mut())
                .ok_or_else(missing)?;
            c.hp = next_hp.0;
            if let CombatRole::Player {
                progression: Some(p),
                ..
            } = &mut c.role
            {
                p.class_state.cp = 1;
            }
            let side = self.sidecar_mut(target.entity)?;
            side.fractions.hp = next_hp.1;
            if player {
                side.fractions.cp = 0;
            }
            if next_hp != (old_hp, old_fraction) {
                self.phase3
                    .as_mut()
                    .ok_or_else(missing)?
                    .vitals_dirty
                    .insert(target.entity);
            }
        }
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Physical,
            outcome: if outcome == LethalOutcome::None {
                SkillOutcome::Miss
            } else {
                SkillOutcome::Hit
            },
            rolled: 0,
            cp_absorbed: 0,
            hp_removed: old_hp.saturating_sub(next_hp.0),
            heal: 0,
            target_hp_after: next_hp.0,
            applied_effect_ids: Vec::new(),
        })
    }
}
