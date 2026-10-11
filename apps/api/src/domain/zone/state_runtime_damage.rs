//! Live skill damage callers. Their RNG order is distinct from auto-attacks.
use super::*;

impl ZoneState {
    fn npc_penalty_v5(
        &self,
        caster: EntityId,
        target: EntityId,
        path: crate::domain::zone::skill_npc::NpcPenaltyPath,
    ) -> Result<ExactRatio, TickError> {
        use crate::domain::zone::skill_npc::NpcPenaltyContext;
        let source = self.entities.get(&caster).ok_or_else(missing)?;
        let victim = self.entities.get(&target).ok_or_else(missing)?;
        let context = NpcPenaltyContext {
            target_level: victim.combat.as_ref().ok_or_else(missing)?.sheet.level(),
            acting_player_level: if source.kind == EntityKind::Player {
                Some(source.combat.as_ref().ok_or_else(missing)?.sheet.level())
            } else {
                None
            },
            attackable: victim.kind == EntityKind::Npc && victim.targeting.attackable,
            l2attackable: victim.kind == EntityKind::Npc && victim.combat.is_some(),
            // The current admitted NPC profile has no raid/minion kind (Phase3 scope).
            raid: false,
            raid_minion: false,
        };
        self.phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .rules
            .npc()
            .penalty
            .multiplier(context, path)
            .map_err(invariant)
    }
    fn damage_stat_v5(
        &self,
        id: EntityId,
        stats: &EffectiveStats,
        name: &str,
        initial: i64,
    ) -> Result<ExactRatio, TickError> {
        q(stats
            .calculator
            .fold(
                name,
                initial,
                FunctionConditions {
                    weapon_kind: if self.sidecar(id)?.player.is_some() {
                        "SWORD"
                    } else {
                        "NONE"
                    },
                    can_resurrect: true,
                },
            )
            .map_err(invariant)?)
        .map_err(invariant)
    }

    fn spread_v5(&mut self, radius: u32) -> Result<ExactRatio, TickError> {
        let spread = if radius == 0 {
            0
        } else {
            i64::from(
                self.roll_below(
                    radius
                        .checked_mul(2)
                        .and_then(|v| v.checked_add(1))
                        .ok_or_else(missing)?,
                ),
            )
            .checked_sub(i64::from(radius))
            .ok_or_else(missing)?
        };
        ExactRatio::new(i128::from(100_i64.checked_add(spread).ok_or_else(missing)?), 100)
            .map_err(invariant)
    }

    /// Formulas.java:1879-1911, TraitType.java:41-78. Multiply type-2 traits in source
    /// enum order, not lexical order; type-3 physical/magical weaknesses are excluded.
    fn attack_traits_v5(a: &EffectiveStats, b: &EffectiveStats) -> Result<ExactRatio, TickError> {
        let weapon = b.traits.get("SWORD").cloned().unwrap_or_default();
        let weapon = ExactRatio::whole(2)
            .checked_sub(weapon.defence)
            .map_err(invariant)?;
        if weapon.parts().0 == 0 {
            return Ok(weapon);
        }
        let mut weakness = ExactRatio::whole(1);
        for name in [
            "BUG_WEAKNESS",
            "ANIMAL_WEAKNESS",
            "PLANT_WEAKNESS",
            "BEAST_WEAKNESS",
            "DRAGON_WEAKNESS",
            "GIANT_WEAKNESS",
            "CONSTRUCT_WEAKNESS",
            "VALAKAS",
            "ANESTHESIA",
            "DEMONIC_WEAKNESS",
            "DIVINE_WEAKNESS",
            "ELEMENTAL_WEAKNESS",
            "FAIRY_WEAKNESS",
            "HUMAN_WEAKNESS",
            "HUMANOID_WEAKNESS",
            "UNDEAD_WEAKNESS",
        ] {
            weakness = weakness
                .checked_mul(Self::trait_modifier_v5(a, b, name)?)
                .map_err(invariant)?;
            if weakness.parts().0 == 0 {
                return Ok(weakness);
            }
        }
        weapon
            .checked_mul(weakness)
            .and_then(|v| v.clamp(ExactRatio::new(1, 20)?, ExactRatio::whole(2)))
            .map_err(invariant)
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Preserve the physical caller's pre/post recipient order.
    pub(super) fn physical_handler_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
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
                .and_then(|e| e.combat.as_ref())
                .ok_or_else(missing)?
                .hp,
            applied_effect_ids: Vec::new(),
        };
        let chance = handler
            .params
            .get("criticalChance")
            .and_then(|p| p.at(rank, skill.max_level))
            .map_or(Ok(0), |v| v.parse::<i32>())
            .map_err(invariant)?;
        let evasion = self.damage_stat_v5(target.entity, &b, "pSkillEvas", 0)?;
        let (_, strength) = self.base_property_v5(caster.entity, "STR")?;
        let Some(critical) = skill_math::physical_skill_checks(
            skill.is_magic == 1,
            skill.abnormal.as_ref().is_some_and(|a| a.is_debuff),
            evasion,
            chance,
            strength,
            |bound| Ok(self.roll_below(bound)),
        )
        .map_err(invariant)?
        else {
            return Ok(result);
        };
        let power = handler
            .params
            .get("power")
            .and_then(|p| p.at(rank, skill.max_level))
            .map_or(Ok(ExactRatio::whole(0)), |s| ExactRatio::parse_literal(s))
            .map_err(invariant)?;
        // Formulas.java:774-809: no shots/shield/facing equipment in Phase 3.
        // Penalty uses acting-player level and precedes the max(1) and caller critical.
        // physicalSkillPower has no supported provider in the capability manifest.
        let damage = skill_math::physical_skill_base(
            power,
            q(a.physical.p_atk().raw()).map_err(invariant)?,
            q(b.physical.p_def().raw()).map_err(invariant)?,
            ExactRatio::whole(1),
            false,
        )
        .map_err(invariant)?
        .checked_mul(Self::attack_traits_v5(&a, &b)?)
        .map_err(invariant)?
        .checked_mul(skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?)
        .map_err(invariant)?
        .checked_mul(self.spread_v5(a.physical.random_damage())?)
        .map_err(invariant)?
        .checked_mul(self.npc_penalty_v5(
            caster.entity,
            target.entity,
            crate::domain::zone::skill_npc::NpcPenaltyPath::PhysicalSkill { critical },
        )?)
        .map_err(invariant)?;
        let damage = if damage
            .checked_cmp(ExactRatio::whole(1))
            .map_err(invariant)?
            .is_lt()
        {
            ExactRatio::whole(1)
        } else {
            damage
        };
        let damage = damage
            .checked_mul(ExactRatio::whole(if critical { 2 } else { 1 }))
            .map_err(invariant)?;
        result.rolled = u32::try_from(damage.trunc().map_err(invariant)?).map_err(invariant)?;
        let damage = self.apply_damage_v5(
            tick,
            DamageRequest {
                source: self.damage_source_v5(caster)?,
                target,
                rolled_damage: result.rolled,
                cause: DamageCause::Direct,
            },
            events,
        )?;
        result.cp_absorbed = damage.cp_absorbed;
        result.hp_removed = damage.hp_removed;
        result.target_hp_after = damage.hp_after;
        result.outcome = if critical {
            SkillOutcome::Crit
        } else {
            SkillOutcome::Hit
        };
        // PhysicalAttack.java:104 / Formulas.java:1562-1569: this is AFTER damage,
        // including lethal damage. The supported manifest has no vengeance provider.
        if skill.is_magic != 1
            && *skill
                .cast_range
                .at(rank, skill.max_level)
                .ok_or_else(missing)?
                <= 40
        {
            self.roll_below(100);
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn magic_handler_v5(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        let a = self.effective(caster.entity)?;
        let b = self.effective(target.entity)?;
        let critical = self.roll_below(1000) < a.magic_crit;
        let magic_level = *skill
            .magic_level
            .get(usize::try_from(rank.saturating_sub(1)).map_err(invariant)?)
            .ok_or_else(missing)?;
        let difference = i32::try_from(b.physical.level())
            .map_err(invariant)?
            .checked_sub(if magic_level > 0 {
                i32::try_from(magic_level).map_err(invariant)?
            } else {
                i32::try_from(a.physical.level()).map_err(invariant)?
            })
            .ok_or_else(missing)?;
        let rate = skill_math::magic_success_rate(
            difference,
            self.npc_penalty_v5(
                caster.entity,
                target.entity,
                crate::domain::zone::skill_npc::NpcPenaltyPath::MagicFailure,
            )?,
            self.damage_stat_v5(target.entity, &b, "magicSuccRes", Q)?,
        )
        .map_err(invariant)?;
        let success = i64::from(self.roll_below(100)) < i64::from(rate);
        let resolution = if success {
            MagicResolution::Success
        } else if i64::from(self.roll_below(100)) < i64::from(rate)
            && b.physical.level().saturating_sub(a.physical.level()) <= 9
        {
            MagicResolution::Half
        } else {
            MagicResolution::Resist
        };
        let spread = self.spread_v5(a.physical.random_damage())?;
        let power = ExactRatio::parse_literal(
            handler
                .params
                .get("power")
                .and_then(|p| p.at(rank, skill.max_level))
                .ok_or_else(missing)?,
        )
        .map_err(invariant)?;
        let damage = skill_math::magical_skill_base(
            power,
            q(a.m_atk_q).map_err(invariant)?,
            q(b.m_def_q).map_err(invariant)?,
            resolution,
            critical,
            self.sidecar(target.entity)?.player.is_some(),
        )
        .map_err(invariant)?
        .checked_mul(spread).map_err(invariant)?
        // Unelemented starter equipment returns 1 before skill attributePower is read.
        .checked_mul(skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?)
        .map_err(invariant)?
        .checked_mul(self.npc_penalty_v5(caster.entity, target.entity, crate::domain::zone::skill_npc::NpcPenaltyPath::Magical)?)
        .map_err(invariant)?;
        let positive = damage
            .checked_cmp(ExactRatio::whole(0))
            .map_err(invariant)?
            == std::cmp::Ordering::Greater;
        let rolled = if positive {
            u32::try_from(damage.floor_scaled(1).map_err(invariant)?).map_err(invariant)?
        } else {
            0
        };
        // MagicalAttackRange.java:83,89: attack-break (idle -> no draw), then vengeance
        // even at zero chance, then recipient status/stun/resource/death processing.
        if matches!(self.sidecar(target.entity)?.cast, CastState::Casting(_)) {
            return Err(missing());
        }
        let result = if positive {
            self.roll_below(100);
            self.apply_damage_v5(
                tick,
                DamageRequest {
                    source: self.damage_source_v5(caster)?,
                    target,
                    rolled_damage: rolled,
                    cause: DamageCause::Direct,
                },
                events,
            )?
        } else {
            DamageOutcome {
                cp_absorbed: 0,
                hp_removed: 0,
                lethal: false,
                hp_after: self
                    .entities
                    .get(&target.entity)
                    .and_then(|e| e.combat.as_ref())
                    .ok_or_else(missing)?
                    .hp,
            }
        };
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Magical,
            outcome: match resolution {
                MagicResolution::Half => SkillOutcome::Half,
                MagicResolution::Resist => SkillOutcome::Resist,
                MagicResolution::Success if critical => SkillOutcome::Crit,
                MagicResolution::Success => SkillOutcome::Hit,
            },
            rolled,
            cp_absorbed: result.cp_absorbed,
            hp_removed: result.hp_removed,
            target_hp_after: result.hp_after,
            heal: 0,
            applied_effect_ids: Vec::new(),
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm
)]
mod tests {
    use super::*;
    use crate::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, ClassState,
    };
    use crate::domain::class::ClassId;
    use crate::domain::{AccountId, Race};
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        rules_data::{load_rules, RulesSource},
        skill_data::{load_phase3_rules, Phase3DataSource},
        zone_data::{parse_zone, TEST_ZONE_TOML},
    };
    use uuid::Uuid;

    fn fixture() -> (ZoneState, TargetBinding, TargetBinding, TargetBinding) {
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let npc =
            NpcCombat::from_template(&stats, &parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0])
                .unwrap();
        let mut zone = ZoneState::new_phase3(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 67,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            stats,
            load_classes(&ClassSource::embedded()).unwrap().registry,
            load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
        )
        .unwrap();
        let mut commands = Vec::new();
        for (n, name) in [(1, "First"), (2, "Second")] {
            let mut load = PlayerLoad::fresh("human_mystic");
            load.progression = Some(Box::new(crate::domain::zone::PlayerProgression {
                identity: CharacterIdentity {
                    account_id: AccountId::from_uuid(Uuid::from_u128(n)),
                    race: Race::Human,
                    base_class_id: ClassId(10),
                    appearance: CharacterAppearance::default(),
                },
                class_state: ClassState::new(ClassId(10)),
                max_cp: 0,
            }));
            commands.push(ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: EntityId::from_uuid(Uuid::from_u128(n)),
                name: name.into(),
                pos: Vec2Fixed::from_tiles(10, 10),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load)),
            }));
        }
        commands.push(ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Keltir".into(),
            pos: Vec2Fixed::from_tiles(11, 10),
            speed: parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0].move_speed,
            combat: Some(Box::new(npc)),
        }));
        assert!(zone
            .run_tick(zone.draft(commands))
            .unwrap()
            .dispositions
            .is_empty());
        let first = zone
            .binding(EntityId::from_uuid(Uuid::from_u128(1)))
            .unwrap();
        let second = zone
            .binding(EntityId::from_uuid(Uuid::from_u128(2)))
            .unwrap();
        let npc = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Npc)
            .unwrap()
            .id;
        let npc = zone.binding(npc).unwrap();
        (zone, first, second, npc)
    }

    fn set_resources(zone: &mut ZoneState, target: TargetBinding, hp: u32, cp: u32) {
        let c = zone
            .entities
            .get_mut(&target.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        c.hp = hp;
        if let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        {
            p.class_state.cp = cp;
        }
    }

    #[test]
    fn unified_live_recipient_uses_source_kind_and_nonlethal_limit_before_cp() {
        let (mut zone, player, target, npc) = fixture();
        for (source, cause, hp, cp, removed_cp, removed_hp) in [
            (player, DamageCause::Direct, 50, 20, 20, 5),
            (npc, DamageCause::Direct, 50, 20, 0, 25),
            (player, DamageCause::Periodic, 10, 20, 9, 0),
            (npc, DamageCause::Periodic, 10, 20, 0, 9),
            (player, DamageCause::Periodic, 1, 20, 0, 0),
        ] {
            set_resources(&mut zone, target, hp, cp);
            let before = zone.rng.clone();
            let outcome = zone
                .apply_damage_v5(
                    Tick(1),
                    DamageRequest {
                        source: zone.damage_source_v5(source).unwrap(),
                        target,
                        cause,
                        rolled_damage: 25,
                    },
                    &mut Vec::new(),
                )
                .unwrap();
            assert_eq!((outcome.cp_absorbed, outcome.hp_removed), (removed_cp, removed_hp));
            assert_eq!(outcome.hp_after, hp.checked_sub(removed_hp).unwrap());
            assert_eq!(zone.rng, before, "unstunned recipient has no intrinsic draw");
            let CombatRole::Player {
                progression: Some(p),
                ..
            } = &zone.entities[&target.entity].combat.as_ref().unwrap().role
            else {
                panic!("player")
            };
            assert_eq!(p.class_state.cp, cp.checked_sub(removed_cp).unwrap());
            assert!(!zone.hate.contains_key(&target.entity));
        }
    }

    #[test]
    fn departed_dot_attribution_never_borrows_a_new_lifes_kind_position_or_credit() {
        let (mut zone, player, target, npc) = fixture();
        for source in [player, npc] {
            let attribution = zone.damage_source_v5(source).unwrap();
            // A new life with the same ID stands in peace. The old effect keeps its source.
            let e = zone.entities.get_mut(&source.entity).unwrap();
            e.combat.as_mut().unwrap().incarnation = 2;
            e.pos = Vec2Fixed::from_tiles(126, 126);
            set_resources(&mut zone, target, 50, 20);
            let request = DamageRequest {
                source: attribution,
                target,
                cause: DamageCause::Periodic,
                rolled_damage: 7,
            };
            assert!(zone
                .damage_classification_v5(
                    Tick(1),
                    DamageRequest {
                        cause: DamageCause::Direct,
                        ..request
                    }
                )
                .is_err());
            let outcome = zone
                .apply_damage_v5(Tick(1), request, &mut Vec::new())
                .unwrap();
            assert_eq!(
                (outcome.cp_absorbed, outcome.hp_removed),
                if source == player { (7, 0) } else { (0, 7) }
            );
            assert!(zone.phase3.as_ref().unwrap().landed_hits.is_empty());
        }
    }

    #[test]
    fn npc_damage_hate_uses_rolled_amount_and_attacked_notification_before_lethal_cleanup() {
        let (mut zone, player, _, npc) = fixture();
        set_resources(&mut zone, npc, 1, 0);
        let mut events = Vec::new();
        let outcome = zone
            .apply_damage_v5(
                Tick(1),
                DamageRequest {
                    source: zone.damage_source_v5(player).unwrap(),
                    target: npc,
                    cause: DamageCause::Direct,
                    rolled_damage: 25,
                },
                &mut events,
            )
            .unwrap();
        assert_eq!(outcome.hp_removed, 1);
        let hates: Vec<_> = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                ZoneEvent::HateChanged { hate, damage, .. } => Some((i, *hate, *damage)),
                _ => None,
            })
            .collect();
        assert_eq!((hates[0].1, hates[0].2), (1, 0));
        assert_eq!((hates[1].1, hates[1].2), (313, 25)); // 1 + floor(25*100/(1+7))
        let death = events
            .iter()
            .position(|e| matches!(e, ZoneEvent::EntityDied { .. }))
            .unwrap();
        assert!(hates[1].0 < death);
        assert_eq!((hates[2].1, hates[2].2), (0, 0));
        assert!(!zone.hate.contains_key(&npc.entity));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ZoneEvent::SpGained { amount: 2, .. }))
                .count(),
            1
        );
        zone.note_hits_v5(Tick(1)).unwrap();
        assert!(zone.phase3.as_ref().unwrap().landed_hits.is_empty());
    }
}
