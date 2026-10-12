//! Live skill damage callers. Their RNG order is distinct from auto-attacks.
#[path = "state_runtime_auto.rs"]
mod auto;
#[path = "state_runtime_blow.rs"]
mod blow;
use super::*;

impl ZoneState {
    /// Formulas.java:708–710,767–770,880–883: player-versus-player only, before
    /// caller truncation. Historical epochs retain their admitted multiplier of one.
    fn item_pvp_damage_bonus(
        &self,
        source: EntityId,
        target: EntityId,
        stats: &EffectiveStats,
        stat: &str,
    ) -> Result<ExactRatio, TickError> {
        if self.phase4.is_none()
            || self.sidecar(source)?.player.is_none()
            || self.sidecar(target)?.player.is_none()
        {
            return Ok(ExactRatio::whole(1));
        }
        self.damage_stat_v5(source, stats, stat, Q)
    }

    fn project_damage_v5(&self, tick: Tick, request: DamageRequest) -> Result<u32, TickError> {
        let c = self
            .entities
            .get(&request.target.entity)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        let cp = match &c.role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => Some(p.class_state.cp),
            CombatRole::Player {
                progression: None, ..
            }
            | CombatRole::Npc { .. } => None,
        };
        state_damage::projected_absorption(
            DamageResources {
                hp_remainder_q: if self.phase4.is_some() {
                    Some(self.sidecar(request.target.entity)?.fractions.hp)
                } else {
                    None
                },
                hp: c.hp,
                mp: c.mp,
                cp,
            },
            DamageReduction {
                cause: request.cause,
                source_kind: request.source.kind,
                classification: self.damage_classification_v5(tick, request)?,
                awake: true,
                rolled_damage: request.rolled_damage,
            },
        )
        .map_err(invariant)
    }

    /// Formulas.java:992-1025 then L2Character.java:4191-4198: consume the chance draw
    /// before checking the cast kind/cutoff. `ATTACK_CANCEL` has no admitted modifier provider.
    fn attack_break_v5(
        &mut self,
        tick: Tick,
        target: TargetBinding,
        absorbed: u32,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
        if self.binding(target.entity) != Some(target) {
            return Ok(());
        }
        let CastState::Casting(cast) = self.sidecar(target.entity)?.cast.clone() else {
            return Ok(());
        };
        let (_, men) = self.base_property_v5(target.entity, "MEN")?;
        let chance = skill_math::attack_break_base(absorbed, men)
            .and_then(|v| v.clamp(ExactRatio::whole(1), ExactRatio::whole(99)))
            .map_err(invariant)?;
        let roll = draw(self, DamageRollSite::AttackBreak, target, 100).map_err(invariant)?;
        let success = chance
            .checked_cmp(ExactRatio::whole(i64::from(roll)))
            .map_err(invariant)?
            .is_gt();
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        let skill = rules.skills().get(cast.skill.id).ok_or_else(missing)?;
        if let Some(cast) = self
            .sidecar_mut(target.entity)?
            .interrupt_cast(tick, success, skill)
        {
            events.push(ZoneEvent::SkillCastCancelled {
                tick,
                caster: target,
                cast_id: cast.id,
                reason: CastCancelReason::Damage,
            });
            self.cancel_item_casts_v6(tick, target.entity, None, CastCancelReason::Damage, events);
        }
        Ok(())
    }

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
                    weapon_kind: self.item_weapon_kind(
                        id,
                        if self.sidecar(id)?.player.is_some() {
                            "SWORD"
                        } else {
                            "NONE"
                        },
                    )?,
                    can_resurrect: true,
                },
            )
            .map_err(invariant)?)
        .map_err(invariant)
    }

    fn spread_v5(
        &mut self,
        radius: u32,
        target: TargetBinding,
        draw: &mut DamageRoll<'_>,
    ) -> Result<ExactRatio, TickError> {
        let spread = if radius == 0 {
            0
        } else {
            i64::from(
                draw(
                    self,
                    DamageRollSite::Spread,
                    target,
                    radius
                        .checked_mul(2)
                        .and_then(|v| v.checked_add(1))
                        .ok_or_else(missing)?,
                )
                .map_err(invariant)?,
            )
            .checked_sub(i64::from(radius))
            .ok_or_else(missing)?
        };
        ExactRatio::new(i128::from(100_i64.checked_add(spread).ok_or_else(missing)?), 100)
            .map_err(invariant)
    }

    /// Formulas.java:1879-1911, TraitType.java:41-78. Multiply type-2 traits in source
    /// enum order, not lexical order; type-3 physical/magical weaknesses are excluded.
    fn attack_traits_v5(
        a: &EffectiveStats,
        b: &EffectiveStats,
        weapon: &str,
    ) -> Result<ExactRatio, TickError> {
        let weapon = b.traits.get(weapon).cloned().unwrap_or_default();
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
        shots: u8,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        self.physical_handler_rolled_v5(
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

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Source caller with injectable rolls, same production path.
    fn physical_handler_rolled_v5(
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
            |bound| {
                draw(
                    self,
                    if bound == 100 {
                        DamageRollSite::PhysicalEvasion
                    } else {
                        DamageRollSite::PhysicalCritical
                    },
                    target,
                    bound,
                )
            },
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
        let pvp_bonus =
            self.item_pvp_damage_bonus(caster.entity, target.entity, &a, "pvpPhysSkillsDmg")?;
        // Formulas.java:774-809: no shots/shield/facing equipment in Phase 3.
        // Penalty uses acting-player level and precedes the max(1) and caller critical.
        // physicalSkillPower has no supported provider in the capability manifest.
        let damage = skill_math::physical_skill_base(
            power,
            q(a.physical.p_atk().raw()).map_err(invariant)?,
            q(b.physical.p_def().raw()).map_err(invariant)?,
            ExactRatio::whole(1),
            shots & 1 != 0,
        )
        .map_err(invariant)?
        .checked_mul(pvp_bonus)
        .map_err(invariant)?
        .checked_mul(Self::attack_traits_v5(
            &a,
            &b,
            self.item_weapon_kind(caster.entity, "SWORD")?,
        )?)
        .map_err(invariant)?
        .checked_mul(skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?)
        .map_err(invariant)?
        .checked_mul(self.spread_v5(a.physical.random_damage(), target, draw)?)
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
        if self.evidence.is_some() {
            let mut detail = serde_json::json!({
                "formula": "calcSkillPhysDam", "skill_id": skill.id, "rank": rank,
                "target": target, "power": power, "p_atk_q": a.physical.p_atk().raw(),
                "p_def_q": b.physical.p_def().raw(), "critical": critical,
                "exact_damage": damage, "rolled_damage": result.rolled
            });
            if self.phase4.is_some() {
                detail
                    .as_object_mut()
                    .ok_or_else(missing)?
                    .insert("shots".into(), serde_json::json!(shots));
            }
            self.evidence_fact("damage_formula", caster.entity, detail);
        }
        let damage = self.apply_damage_rolled_v5(
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
            draw(self, DamageRollSite::PhysicalVengeance, target, 100).map_err(invariant)?;
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
        shots: u8,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        self.magic_handler_rolled_v5(
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

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Source caller with injectable rolls, same production path.
    fn magic_handler_rolled_v5(
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
        let a = self.effective(caster.entity)?;
        let b = self.effective(target.entity)?;
        let critical = draw(self, DamageRollSite::MagicCritical, target, 1000)
            .map_err(invariant)?
            < a.magic_crit;
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
        let success =
            i64::from(draw(self, DamageRollSite::MagicSuccess, target, 100).map_err(invariant)?)
                < i64::from(rate);
        let resolution = if success {
            MagicResolution::Success
        } else if self.sidecar(caster.entity)?.player.is_some()
            && i64::from(draw(self, DamageRollSite::MagicFailure, target, 100).map_err(invariant)?)
                < i64::from(rate)
            && b.physical.level().saturating_sub(a.physical.level()) <= 9
        {
            MagicResolution::Half
        } else {
            MagicResolution::Resist
        };
        let spread = self.spread_v5(a.physical.random_damage(), target, draw)?;
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
            q(a.m_atk_q).and_then(|value| value.checked_mul(ExactRatio::whole(if shots & 4 != 0 {4} else if shots & 2 != 0 {2} else {1}))).map_err(invariant)?,
            q(b.m_def_q).map_err(invariant)?,
            resolution,
            critical,
            self.sidecar(caster.entity)?.player.is_some() && self.sidecar(target.entity)?.player.is_some(),
        )
        .map_err(invariant)?
        .checked_mul(spread).map_err(invariant)?
        .checked_mul(self.item_pvp_damage_bonus(caster.entity, target.entity, &a,
            if skill.is_magic == 1 { "pvpMagicalDmg" } else { "pvpPhysSkillsDmg" })?).map_err(invariant)?
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
        if self.evidence.is_some() {
            let mut detail = serde_json::json!({
                "formula": "calcMagicDam", "skill_id": skill.id, "rank": rank,
                "target": target, "power": power, "m_atk_q": a.m_atk_q, "m_def_q": b.m_def_q,
                "magic_level_difference": difference, "magic_success_rate": rate,
                "critical": critical, "resolution": format!("{resolution:?}"),
                "spread": spread, "exact_damage": damage, "rolled_damage": rolled
            });
            if self.phase4.is_some() {
                detail
                    .as_object_mut()
                    .ok_or_else(missing)?
                    .insert("shots".into(), serde_json::json!(shots));
            }
            self.evidence_fact("damage_formula", caster.entity, detail);
        }
        // MagicalAttackRange.java:83,89: attack-break precedes vengeance, and both
        // precede recipient status/CP/HP. Whole-zero positive source damage keeps the tail.
        let result = if positive {
            let request = DamageRequest {
                source: self.damage_source_v5(caster)?,
                target,
                rolled_damage: rolled,
                cause: DamageCause::Direct,
            };
            let absorbed = self.project_damage_v5(tick, request)?;
            self.attack_break_v5(tick, target, absorbed, events, draw)?;
            // The executable manifest has no vengeance provider: retain its zero-chance draw.
            draw(self, DamageRollSite::MagicVengeance, target, 100).map_err(invariant)?;
            self.apply_damage_rolled_v5(tick, request, events, draw)?
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
impl ZoneState {
    pub(crate) fn item_checkpoint_fixture() -> (Self, TargetBinding, TargetBinding) {
        tests::equipment_fixture()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
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
        fixture_with_order(false)
    }
    fn fixture_with_order(
        reverse: bool,
    ) -> (ZoneState, TargetBinding, TargetBinding, TargetBinding) {
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
        let mut players = [(1, "First"), (2, "Second")];
        if reverse {
            players.reverse();
        }
        for (n, name) in players {
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

    #[test]
    fn low_rank_magic_against_level_36_completes_with_required_resistance_draws() {
        let (mut zone, source, target, _) = fixture();
        let target_xp = zone.rules.as_ref().unwrap().xp_to_level(36).unwrap();
        if let CombatRole::Player { xp, .. } = &mut zone
            .entities
            .get_mut(&target.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        {
            *xp = target_xp;
        }
        zone.relevel(target.entity).unwrap();
        zone.sync_passives_v5(target.entity).unwrap();
        zone.rebuild_v5(target.entity).unwrap();
        start_cast(&mut zone, source, target);
        let CastState::Casting(cast) = zone.sidecar(source.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        while zone.next_tick < cast.complete_at {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        let before = zone.rng.get_word_pos();
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        let rows = tick
            .events
            .iter()
            .find_map(|event| match event {
                ZoneEvent::SkillCastResult { targets, .. } => Some(targets),
                _ => None,
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, SkillOutcome::Resist);
        // Magic crit, success1, player success2, spread, zero-chance vengeance.
        assert_eq!(zone.rng.get_word_pos() - before, 5);
    }

    #[test]
    fn derived_cache_and_due_indexes_match_cold_reconstruction_after_every_tick() {
        let (mut warm, caster, _, npc) = fixture();
        start_cast(&mut warm, caster, npc);
        let mut cold = ZoneState::from_snapshot(warm.snapshot()).unwrap();
        let cached = warm.effective(caster.entity).unwrap();
        assert!(Arc::ptr_eq(&cached, &warm.effective(caster.entity).unwrap()));
        for _ in 0..90 {
            let actual = warm.run_tick(warm.draft(Vec::new())).unwrap();
            let rebuilt = cold.run_tick(cold.draft(Vec::new())).unwrap();
            assert_eq!(actual, rebuilt);
            let phase = warm.phase3.as_ref().unwrap();
            assert_eq!(
                phase.schedule,
                crate::domain::zone::state_effects::Phase3Schedule::rebuild(
                    &phase.canonical.entities
                )
                .unwrap()
            );
            for (id, side) in &phase.canonical.entities {
                assert_eq!(
                    *warm.effective(*id).unwrap(),
                    warm.effective_for(&warm.entities[id], side).unwrap()
                );
            }
            cold = ZoneState::from_snapshot(cold.snapshot()).unwrap();
        }
    }

    #[test]
    fn live_pvp_lethal_truth_table_and_cold_cast_replay() {
        for (flagged, karma, pk, pvp) in [
            (false, 0, 1, 0),
            (true, 0, 0, 1),
            (false, 100, 0, 0),
            (true, 100, 0, 1),
        ] {
            let (mut zone, source, target, _) = fixture();
            let victim = zone
                .sidecar_mut(target.entity)
                .unwrap()
                .player
                .as_mut()
                .unwrap();
            victim.pvp.karma = karma;
            victim.pvp.flag_until = flagged.then_some(Tick(3000));
            set_resources(&mut zone, target, 1, 0);
            start_cast(&mut zone, source, target);
            let mut replay = ZoneState::from_snapshot(zone.snapshot()).unwrap();
            let mut events = Vec::new();
            for _ in 0..50 {
                let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
                assert_eq!(tick, replay.run_tick(replay.draft(Vec::new())).unwrap());
                events.extend(tick.events);
                if zone.entities[&target.entity].targeting.dead {
                    break;
                }
            }
            assert!(zone.entities[&target.entity].targeting.dead);
            let killer = &zone
                .sidecar(source.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .pvp;
            assert_eq!((killer.pk_count, killer.pvp_count), (pk, pvp));
            assert_eq!(killer.karma > 0, pk > 0);
            assert_eq!(killer.flag_until.is_some(), flagged || karma == 0);
            let dead = zone
                .sidecar(target.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap();
            assert_eq!(dead.death_penalty_level, 0);
            assert!(dead.last_death.is_some());
            if pk != 0 || pvp != 0 {
                assert!(events.iter().any(|e| matches!(e, ZoneEvent::PvpStateChanged { entity, .. } if *entity == source.entity)));
            }
        }
    }

    #[test]
    fn npc_auto_runs_from_hate_request_and_restores_pending_swing() {
        let (mut zone, player, _, npc) = fixture();
        zone.apply_damage_v5(
            Tick(0),
            DamageRequest {
                source: zone.damage_source_v5(player).unwrap(),
                target: npc,
                cause: DamageCause::Direct,
                rolled_damage: 1,
            },
            &mut Vec::new(),
        )
        .unwrap();
        let mut started = false;
        let mut impacted = false;
        for _ in 0..50 {
            let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
            if tick.events.iter().any(|e| matches!(e, ZoneEvent::AttackStarted { attacker, phase3: Some(_), .. } if *attacker == npc.entity)) {
                started = true;
                let mut restored = ZoneState::from_snapshot(zone.snapshot()).unwrap();
                for _ in 0..20 {
                    let live = zone.run_tick(zone.draft(Vec::new())).unwrap();
                    assert_eq!(live, restored.run_tick(restored.draft(Vec::new())).unwrap());
                    if live.events.iter().any(|e| matches!(e, ZoneEvent::AttackResult { attacker, phase3: Some(p), .. } if *attacker == npc.entity && p.cp_absorbed == 0)) { impacted = true; break; }
                }
                break;
            }
        }
        assert!(started && impacted);
    }

    #[test]
    fn auto_rng_recipient_stun_precedes_post_damage_cast_break_and_miss_skips_tail() {
        for miss in [false, true] {
            let (mut zone, caster, _, npc) = fixture();
            start_cast(&mut zone, caster, npc);
            add_stun(&mut zone, caster);
            let now = zone.next_tick;
            let pos = zone.entities[&caster.entity].pos;
            let e = zone.entities.get_mut(&npc.entity).unwrap();
            e.pos = pos;
            e.targeting.target = Some(caster.entity);
            let swing = crate::domain::zone::Swing {
                target: caster.entity,
                target_incarnation: caster.incarnation,
                start: Tick(0),
                impact: now,
                ready: Tick(100),
            };
            e.combat.as_mut().unwrap().swing = Some(swing);
            let radius = zone.entities[&npc.entity]
                .combat
                .as_ref()
                .unwrap()
                .sheet
                .random_damage();
            let expected = if miss {
                vec![(DamageRollSite::AutoHit, 1000, 999)]
            } else {
                vec![
                    (DamageRollSite::AutoHit, 1000, 0),
                    (DamageRollSite::AutoCritical, 1000, 999),
                    (DamageRollSite::Spread, radius * 2 + 1, radius),
                    (DamageRollSite::StunRemoval, 10, 0),
                    (DamageRollSite::AttackBreak, 100, 0),
                ]
            };
            let mut trace = Vec::new();
            let mut events = Vec::new();
            zone.land_auto_v5(
                now,
                npc.entity,
                swing,
                &mut events,
                &mut scripted(&expected, &mut trace),
            )
            .unwrap();
            assert_eq!(trace.len(), expected.len());
            assert_eq!(matches!(zone.sidecar(caster.entity).unwrap().cast, CastState::Idle), !miss);
            assert!(events.iter().any(|e| matches!(e, ZoneEvent::AttackResult { outcome, .. } if (*outcome == crate::domain::zone::AttackOutcome::Miss) == miss)));
        }
    }

    #[test]
    fn flag_expiry_and_peace_intent_are_live_and_do_not_draw_rng() {
        let (mut zone, source, target, _) = fixture();
        let now = zone.next_tick;
        zone.sidecar_mut(source.entity)
            .unwrap()
            .player
            .as_mut()
            .unwrap()
            .pvp
            .flag_until = Some(now);
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert!(tick.events.iter().any(|e| matches!(e, ZoneEvent::PvpStateChanged { entity, pvp, .. } if *entity == source.entity && pvp.flag_until.is_none())));
        let center = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .geometry()
            .center()
            .unwrap();
        let old = zone.entities[&target.entity].pos;
        zone.entities.get_mut(&target.entity).unwrap().pos = center;
        zone.aoi.relocate(target.entity, old, center);
        zone.entities
            .get_mut(&source.entity)
            .unwrap()
            .targeting
            .target = Some(target.entity);
        let cursor = zone.rng.get_word_pos();
        let tick = zone
            .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Attack {
                entity: source.entity,
            })]))
            .unwrap();
        assert_eq!(tick.dispositions[0].reason, RejectReason::PeaceZone);
        assert_eq!(zone.rng.get_word_pos(), cursor);
    }

    #[test]
    fn peace_entry_cancels_pending_auto_and_hostile_cast_before_damage_or_rng() {
        for auto in [false, true] {
            let (mut zone, source, target, npc) = fixture();
            zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn {
                entity: npc.entity,
            })]))
            .unwrap();
            if auto {
                let tick = zone
                    .run_tick(zone.draft(vec![
                        ZoneInput::system(ZoneCommand::SetTarget {
                            entity: source.entity,
                            target: Some(target.entity),
                        }),
                        ZoneInput::system(ZoneCommand::Attack {
                            entity: source.entity,
                        }),
                    ]))
                    .unwrap();
                assert!(tick.dispositions.is_empty());
                assert!(zone.entities[&source.entity]
                    .combat
                    .as_ref()
                    .unwrap()
                    .swing
                    .is_some());
            } else {
                start_cast(&mut zone, source, target);
            }
            let center = zone
                .phase3
                .as_ref()
                .unwrap()
                .canonical
                .rules
                .geometry()
                .center()
                .unwrap();
            let old = zone.entities[&target.entity].pos;
            zone.entities.get_mut(&target.entity).unwrap().pos = center;
            zone.aoi.relocate(target.entity, old, center);
            let hp = zone.entities[&target.entity].combat.as_ref().unwrap().hp;
            let cursor = zone.rng.get_word_pos();
            // Normalize the observer baseline after the test-only boundary relocation.
            zone = ZoneState::from_snapshot(zone.snapshot()).unwrap();
            let mut cold = ZoneState::from_snapshot(zone.snapshot()).unwrap();
            let mut canceled = false;
            for _ in 0..60 {
                let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
                assert_eq!(tick, cold.run_tick(cold.draft(Vec::new())).unwrap());
                assert!(!tick.events.iter().any(|e| matches!(
                    e,
                    ZoneEvent::AttackResult { .. } | ZoneEvent::SkillCastResult { .. }
                )));
                canceled |= tick.events.iter().any(|e| {
                    matches!(
                        e,
                        ZoneEvent::AttackCancelled { .. } | ZoneEvent::SkillCastCancelled { .. }
                    )
                });
            }
            assert!(canceled);
            assert_eq!(zone.entities[&target.entity].combat.as_ref().unwrap().hp, hp);
            assert_eq!(zone.rng.get_word_pos(), cursor);
        }
    }

    #[test]
    fn swing_hydration_hides_unknown_target_after_all_identities_and_stats_remain_private() {
        use crate::interface::{grpc::pb, zone_mapping::observer_output_to_pb};
        let (mut zone, target, observer, npc) = fixture();
        for (binding, pos) in [
            (target, Vec2Fixed::from_tiles(64, 10)),
            (npc, Vec2Fixed::from_tiles(63, 10)),
        ] {
            let old = zone.entities[&binding.entity].pos;
            zone.entities.get_mut(&binding.entity).unwrap().pos = pos;
            zone.aoi.relocate(binding.entity, old, pos);
        }
        zone.apply_damage_v5(
            zone.next_tick,
            DamageRequest {
                source: zone.damage_source_v5(target).unwrap(),
                target: npc,
                cause: DamageCause::Direct,
                rolled_damage: 1,
            },
            &mut Vec::new(),
        )
        .unwrap();
        for _ in 0..20 {
            if zone.entities[&npc.entity]
                .combat
                .as_ref()
                .unwrap()
                .swing
                .is_some()
            {
                break;
            }
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        assert!(zone.entities[&npc.entity]
            .combat
            .as_ref()
            .unwrap()
            .swing
            .is_some());
        // A new observer baseline while the original swing is still in flight.
        zone.known.remove(&observer.entity);
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        let outputs = &tick.outputs[&observer.entity];
        let wires: Vec<_> = outputs
            .iter()
            .flat_map(|o| observer_output_to_pb(o, tick.server_time_ms))
            .filter_map(|e| match e.payload {
                Some(pb::server_message::Payload::Event(event)) => event.event,
                _ => None,
            })
            .collect();
        let spawn = wires.iter().position(|e| matches!(e, pb::world_event::Event::Spawn(s) if s.entity_id == npc.entity.to_string())).unwrap();
        let swing = wires.iter().position(|e| matches!(e, pb::world_event::Event::AttackStarted(s) if s.attacker == npc.entity.to_string())).unwrap();
        assert!(spawn < swing);
        let pb::world_event::Event::AttackStarted(started) = &wires[swing] else {
            panic!("swing")
        };
        assert!(started.baseline);
        assert_eq!(started.baseline_tick, tick.tick.0);
        assert!(started.target.is_empty());
        assert_eq!(started.target_incarnation, 0);
        assert!(wires.iter().all(|e| !matches!(e, pb::world_event::Event::StatsChanged(s) if s.entity != observer.entity.to_string())));
        assert!(!wires.iter().any(|e| matches!(e, pb::world_event::Event::PvpStateChanged(s) if s.entity == target.entity.to_string())));
    }

    #[test]
    fn paid_rank_survives_xp_level_up_and_does_not_block_milestone_supply() {
        let (mut zone, player, _, npc) = fixture();
        let before_twenty = zone.rules.as_ref().unwrap().xp_to_level(20).unwrap() - 1;
        let c = zone
            .entities
            .get_mut(&player.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        if let CombatRole::Player {
            xp,
            progression: Some(p),
            ..
        } = &mut c.role
        {
            *xp = before_twenty;
            p.class_state.learned_skills = vec![crate::domain::subclass::LearnedSkill {
                key: "l2.skill.1177".into(),
                level: 1,
            }];
        }
        zone.relevel(player.entity).unwrap();
        zone.sync_passives_v5(player.entity).unwrap();
        zone.rebuild_v5(player.entity).unwrap();
        let mut events = Vec::new();
        zone.apply_damage_v5(
            zone.next_tick,
            DamageRequest {
                source: zone.damage_source_v5(player).unwrap(),
                target: npc,
                cause: DamageCause::Direct,
                rolled_damage: u32::MAX,
            },
            &mut events,
        )
        .unwrap();
        let c = zone.entities[&player.entity].combat.as_ref().unwrap();
        assert_eq!(c.sheet.level(), 20);
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            panic!("player")
        };
        assert_eq!(p.class_state.token_tier_1_count, 1);
        assert_eq!(p.class_state.milestone_claimed_mask, 1);
        assert!(p
            .class_state
            .learned_skills
            .iter()
            .any(|s| s.key == "l2.skill.1177" && s.level == 1));
        assert!(events
            .iter()
            .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
        assert!(ZoneState::from_snapshot(zone.snapshot()).is_ok());
    }

    fn resurrection_fixture() -> (ZoneState, TargetBinding, TargetBinding) {
        use crate::domain::character_progression::{
            FrozenTransferResult, SuccessfulTransferReceipt,
        };
        let (mut zone, caster, target, npc) = fixture();
        let xp20 = zone.rules.as_ref().unwrap().xp_to_level(20).unwrap();
        for id in [caster.entity, target.entity] {
            let c = zone.entities.get_mut(&id).unwrap().combat.as_mut().unwrap();
            if let CombatRole::Player {
                xp,
                progression: Some(p),
                ..
            } = &mut c.role
            {
                *xp = xp20;
                if id == caster.entity {
                    p.class_state.current_class_id = ClassId(15);
                    p.class_state
                        .successful_transfer_receipts
                        .push(SuccessfulTransferReceipt {
                            key: Uuid::from_u128(400),
                            target_class_id: ClassId(15),
                            result: FrozenTransferResult {
                                character_id: crate::domain::CharacterId::from_uuid(id.as_uuid()),
                                identity: p.identity.clone(),
                                name: crate::domain::CharacterName::new("First").unwrap(),
                                current_class_id: ClassId(15),
                                level: 20,
                                xp: xp20,
                                sp: 0,
                                stats: zone
                                    .classes
                                    .as_ref()
                                    .unwrap()
                                    .get(ClassId(10))
                                    .unwrap()
                                    .base_stats,
                                position_millitiles: [10000, 10000],
                                hp: 1,
                                mp: 1,
                                cp: 0,
                                max_hp: 1,
                                max_mp: 1,
                                max_cp: 0,
                                token_tier_1_count: 0,
                                token_tier_2_count: 0,
                                granted_skill_keys: Vec::new(),
                            },
                        });
                    p.class_state.learned_skills = vec![crate::domain::subclass::LearnedSkill {
                        key: "l2.skill.1016".into(),
                        level: 2,
                    }];
                }
            }
            zone.relevel(id).unwrap();
            zone.sync_passives_v5(id).unwrap();
            zone.rebuild_v5(id).unwrap();
            let c = zone.entities.get_mut(&id).unwrap().combat.as_mut().unwrap();
            c.hp = c.sheet.max_hp();
            c.mp = c.sheet.max_mp();
        }
        zone.apply_damage_v5(
            zone.next_tick,
            DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target,
                cause: DamageCause::Direct,
                rolled_damage: u32::MAX,
            },
            &mut Vec::new(),
        )
        .unwrap();
        (zone, caster, target)
    }

    #[test]
    fn resurrection_cast_offer_decline_accept_town_expiry_and_final_life_are_live() {
        for response in [0, 1, 2, 3] {
            let (mut zone, caster, target) = resurrection_fixture();
            let start = zone
                .run_tick(zone.draft(vec![ZoneInput::session(
                    caster.entity,
                    SessionGeneration(1),
                    1,
                    ZoneCommand::CastSkill {
                        entity: caster.entity,
                        skill_id: crate::domain::zone::skill::SkillId(1016),
                        target: Some(target.entity),
                    },
                )]))
                .unwrap();
            assert!(start.dispositions.is_empty(), "{:?}", start.dispositions);
            let mut replay = ZoneState::from_snapshot(zone.snapshot()).unwrap();
            let mut offered = false;
            for _ in 0..100 {
                let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
                assert_eq!(tick, replay.run_tick(replay.draft(Vec::new())).unwrap());
                if tick.events.iter().any(|e| {
                    matches!(
                        e,
                        ZoneEvent::ResurrectOffer {
                            state: crate::domain::zone::ResurrectOfferState::Offered,
                            ..
                        }
                    )
                }) {
                    offered = true;
                    break;
                }
            }
            assert!(offered);
            let offer = zone
                .sidecar(target.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .resurrect_offer
                .clone()
                .unwrap();
            if response == 3 {
                // Exactly the deadline: acceptance is refused and expiry emits once.
                while zone.next_tick < offer.expires_at {
                    zone.run_tick(zone.draft(Vec::new())).unwrap();
                }
            }
            let command = if response == 2 {
                ZoneCommand::Respawn {
                    entity: target.entity,
                }
            } else {
                ZoneCommand::AcceptResurrect {
                    entity: target.entity,
                    offer_id: offer.id,
                    accept: response != 0,
                }
            };
            let tick = zone
                .run_tick(zone.draft(vec![ZoneInput::session(
                    target.entity,
                    SessionGeneration(1),
                    2,
                    command,
                )]))
                .unwrap();
            assert_eq!(tick.dispositions.is_empty(), response != 3);
            assert!(zone
                .sidecar(target.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .resurrect_offer
                .is_none());
            if response == 1 || response == 2 {
                let c = zone.entities[&target.entity].combat.as_ref().unwrap();
                assert_eq!(c.incarnation, target.incarnation + 1);
                assert!(c.hp > 0);
                assert_eq!(c.protected_until.is_some(), response == 2);
                assert!(tick.outputs[&target.entity].iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::VitalsChanged { entity, incarnation, baseline: Some(at), .. }) if *entity == target.entity && *incarnation == c.incarnation && *at == tick.tick)));
                assert!(zone
                    .sidecar(target.entity)
                    .unwrap()
                    .player
                    .as_ref()
                    .unwrap()
                    .last_death
                    .is_none());
            } else {
                assert!(zone.entities[&target.entity].targeting.dead);
            }
            assert!(ZoneState::from_snapshot(zone.snapshot()).is_ok());
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One replayable live lifecycle, including real reuse waits.
    fn gameplay_recording_exercises_offers_auto_cp_pk_and_npc_xp() {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        use crate::infrastructure::eventlog::Recording;
        fn record(
            zone: &mut ZoneState,
            rec: &mut Recording,
            commands: Vec<ZoneCommand>,
        ) -> AppliedTick {
            let inputs = commands
                .into_iter()
                .map(|command| {
                    ZoneInput::session(
                        command.entity().unwrap(),
                        SessionGeneration(1),
                        u32::try_from(zone.next_tick.0).unwrap(),
                        command,
                    )
                })
                .collect();
            let applied = zone.run_tick(zone.draft(inputs)).unwrap();
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
            rec.records
                .push(AppliedTickRecord::from_applied(ZoneId(1), &applied));
            rec.watermark.last_tick = Some(applied.tick);
            rec.watermark.records = u64::try_from(rec.records.len()).unwrap();
            applied
        }
        let (mut zone, caster, target) = resurrection_fixture();
        let c = zone
            .entities
            .get_mut(&caster.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        else {
            panic!("player")
        };
        p.class_state
            .merge_learned_skills([crate::domain::subclass::LearnedSkill {
                key: "l2.skill.1177".into(),
                level: 5,
            }]);
        zone.sync_passives_v5(caster.entity).unwrap();
        zone.rebuild_v5(caster.entity).unwrap();
        set_resources(&mut zone, target, 0, 5);
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        // The initial corpse is an admitted snapshot; every later mutation is recorded input.
        let snapshot = zone.snapshot();
        for e in &snapshot.entities {
            if let Some(CombatState {
                role:
                    CombatRole::Player {
                        progression: Some(p),
                        ..
                    },
                ..
            }) = &e.combat
            {
                assert!(p.class_state.cp <= p.max_cp, "cp {} max {}", p.class_state.cp, p.max_cp);
                p.class_state
                    .validate_for_phase3(
                        zone.classes.as_ref().unwrap(),
                        &p.identity,
                        crate::domain::CharacterId::from_uuid(e.id.as_uuid()),
                    )
                    .unwrap();
            }
        }
        zone = ZoneState::from_snapshot(snapshot.clone()).unwrap();
        let mut rec = Recording {
            watermark: Watermark {
                zone: snapshot.seed.zone,
                epoch: snapshot.seed.epoch,
                last_tick: None,
                records: 0,
                reason: WatermarkReason::Capture,
            },
            snapshot,
            records: Vec::new(),
        };
        for accept in [false, true] {
            while zone
                .sidecar(caster.entity)
                .unwrap()
                .reuse
                .values()
                .any(|at| *at > zone.next_tick)
            {
                record(&mut zone, &mut rec, vec![]);
            }
            record(
                &mut zone,
                &mut rec,
                vec![ZoneCommand::CastSkill {
                    entity: caster.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1016),
                    target: Some(target.entity),
                }],
            );
            for _ in 0..100 {
                if zone
                    .sidecar(target.entity)
                    .unwrap()
                    .player
                    .as_ref()
                    .unwrap()
                    .resurrect_offer
                    .is_some()
                {
                    break;
                }
                record(&mut zone, &mut rec, vec![]);
            }
            let offer_id = zone
                .sidecar(target.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .resurrect_offer
                .as_ref()
                .unwrap()
                .id;
            record(
                &mut zone,
                &mut rec,
                vec![ZoneCommand::AcceptResurrect {
                    entity: target.entity,
                    offer_id,
                    accept,
                }],
            );
        }
        record(
            &mut zone,
            &mut rec,
            vec![
                ZoneCommand::SetTarget {
                    entity: caster.entity,
                    target: Some(target.entity),
                },
                ZoneCommand::Attack {
                    entity: caster.entity,
                },
            ],
        );
        let mut cp_hit = false;
        for _ in 0..120 {
            let tick = record(&mut zone, &mut rec, vec![]);
            cp_hit |= tick.events.iter().any(|e| matches!(e, ZoneEvent::AttackResult { phase3: Some(p), .. } if p.cp_absorbed > 0));
        }
        assert!(cp_hit);
        for _ in 0..15 {
            if zone.entities[&target.entity].targeting.dead {
                break;
            }
            record(
                &mut zone,
                &mut rec,
                vec![ZoneCommand::CastSkill {
                    entity: caster.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1177),
                    target: Some(target.entity),
                }],
            );
            for _ in 0..60 {
                record(&mut zone, &mut rec, vec![]);
            }
        }
        assert!(zone.entities[&target.entity].targeting.dead);
        assert_eq!(
            zone.sidecar(caster.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .pvp
                .pk_count,
            1
        );
        record(
            &mut zone,
            &mut rec,
            vec![ZoneCommand::Respawn {
                entity: target.entity,
            }],
        );
        let npc = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Npc)
            .unwrap()
            .id;
        record(
            &mut zone,
            &mut rec,
            vec![ZoneCommand::CastSkill {
                entity: caster.entity,
                skill_id: crate::domain::zone::skill::SkillId(1177),
                target: Some(npc),
            }],
        );
        for _ in 0..100 {
            if zone.entities[&npc].targeting.dead {
                break;
            }
            record(&mut zone, &mut rec, vec![]);
        }
        assert!(zone.entities[&npc].targeting.dead);
        let mut cold = ZoneState::from_snapshot(rec.snapshot.clone()).unwrap();
        for expected in &rec.records {
            let actual = cold
                .run_tick(AppliedTickDraft {
                    epoch: expected.epoch,
                    tick: expected.tick,
                    commands: expected.commands.clone(),
                })
                .unwrap();
            assert_eq!(&AppliedTickRecord::from_applied(ZoneId(1), &actual), expected);
        }
        if let Some(path) = std::env::var_os("NIGHTFALL_V5_GAMEPLAY_OUT") {
            rec.write(std::path::Path::new(&path)).unwrap();
        }
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
    #[test]
    fn live_player_death_records_actual_loss_penalty_and_final_resources_once() {
        for roll in [0, 19, 20, 99] {
            let (mut zone, _, target, npc) = fixture();
            let rules = zone.rules.as_ref().unwrap();
            let xp = rules.xp_to_level(20).unwrap();
            if let CombatRole::Player { xp: current, .. } = &mut zone
                .entities
                .get_mut(&target.entity)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .role
            {
                *current = xp;
            }
            zone.relevel(target.entity).unwrap();
            zone.sync_passives_v5(target.entity).unwrap();
            zone.rebuild_v5(target.entity).unwrap();
            set_resources(&mut zone, target, 1, 20);
            let before = zone.entities.clone();
            let mut events = Vec::new();
            let mut trace = Vec::new();
            let request = DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target,
                cause: DamageCause::Direct,
                rolled_damage: 50,
            };
            zone.enable_replay_evidence();
            let expected = [(DamageRollSite::DeathPenalty, 100, roll)];
            let outcome = zone
                .apply_damage_rolled_v5(
                    Tick(1),
                    request,
                    &mut events,
                    &mut scripted(&expected, &mut trace),
                )
                .unwrap();
            assert!(outcome.lethal);
            let evidence = zone.take_replay_evidence();
            let death = evidence.iter().find(|fact| fact.kind == "death_record");
            assert_eq!(
                death.map(|fact| &fact.data["penalty_roll"]),
                Some(&serde_json::json!(roll))
            );
            assert_eq!(
                death.map(|fact| &fact.data["after"]["death_penalty_level"]),
                Some(&serde_json::json!(u8::from(roll < 20)))
            );
            assert_eq!((outcome.hp_removed, outcome.cp_absorbed), (1, 0));
            assert_eq!(trace.len(), 1);
            let c = zone.entities[&target.entity].combat.as_ref().unwrap();
            let CombatRole::Player { xp: after, .. } = c.role else {
                panic!("player")
            };
            let side = zone.sidecar(target.entity).unwrap();
            let p = side.player.as_ref().unwrap();
            assert_eq!(p.death_penalty_level, u8::from(roll < 20));
            assert_eq!(p.last_death.as_ref().unwrap().pre_death_xp, xp);
            assert_eq!(
                p.last_death.as_ref().unwrap().recoverable_death_xp_loss,
                xp.checked_sub(after).unwrap()
            );
            assert!(after < xp);
            assert_eq!(side.fractions, ResourceFractions::default());
            assert_eq!(side.cast, CastState::Idle);
            assert_eq!(c.sheet, zone.effective(target.entity).unwrap().physical);
            let cursor = zone.rng.get_word_pos();
            assert!(zone.apply_damage_v5(Tick(1), request, &mut events).is_err());
            assert_eq!(zone.rng.get_word_pos(), cursor);
            assert_eq!(events.iter().filter(|e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == target.entity)).count(), 1);
            zone.final_resources_v5(Tick(1), &before, &mut events)
                .unwrap();
            assert_eq!(events.iter().filter(|e| matches!(e, ZoneEvent::VitalsChanged { entity, hp: 0, incarnation: 1, .. } if *entity == target.entity)).count(), 1);
        }
    }

    pub(super) fn equipment_fixture() -> (ZoneState, TargetBinding, TargetBinding) {
        use crate::domain::zone::{
            item_angles::FacingVector,
            item_functions::tests::{equip, player},
            item_sources,
            phase4::Phase4Zone,
            ItemInstanceRange, Phase4Snapshot,
        };
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let (mut zone, source, target, npc) = fixture();
        zone.phase3.as_mut().unwrap().canonical.rules = Arc::clone(&bundle.skills);
        zone.phase3.as_mut().unwrap().canonical.rules_hash = bundle.skills.hash();
        zone.meta.schema_version = 10;
        zone.meta.digest_version = StateDigestVersion::BinaryV6;
        zone.meta.phase3_admission_policy =
            Some(crate::domain::zone::Phase3AdmissionPolicy::version_two());
        let mut items =
            Phase4Snapshot::new(&bundle.items, ItemInstanceRange::new(1, 1000, 10000).unwrap());
        for id in [source.entity, target.entity] {
            let mut item_player = player();
            item_player.starter_kit_receipt.source_class = ClassId(10);
            if id == target.entity {
                let next = zone
                    .phase3
                    .as_ref()
                    .unwrap()
                    .canonical
                    .next_install_sequence;
                equip(&mut item_player, 71, 4, crate::domain::zone::EquipSlot::Rhand, next);
                equip(
                    &mut item_player,
                    58,
                    6,
                    crate::domain::zone::EquipSlot::Chest,
                    next.checked_add(1).unwrap(),
                );
                equip(
                    &mut item_player,
                    59,
                    6,
                    crate::domain::zone::EquipSlot::Legs,
                    next.checked_add(2).unwrap(),
                );
                equip(
                    &mut item_player,
                    47,
                    6,
                    crate::domain::zone::EquipSlot::Head,
                    next.checked_add(3).unwrap(),
                );
                zone.phase3
                    .as_mut()
                    .unwrap()
                    .canonical
                    .next_install_sequence = next.checked_add(4).unwrap();
                item_player.weapon_penalty = 2;
                item_player.armor_penalty = 1;
                item_player.overload_level = 2;
                item_player.active_sets =
                    item_sources::reconcile_sets(&item_player, &bundle.items, None).unwrap();
                item_sources::reconcile_sources(
                    &mut item_player,
                    &bundle.items,
                    &mut zone
                        .phase3
                        .as_mut()
                        .unwrap()
                        .canonical
                        .next_install_sequence,
                )
                .unwrap();
            }
            items.players.insert(id, item_player);
        }
        for id in zone.phase3.as_ref().unwrap().canonical.entities.keys() {
            items.facing.insert(*id, FacingVector::EAST);
        }
        zone.phase4 = Some(Phase4Zone {
            canonical: items,
            rules: Arc::clone(&bundle.items),
        });
        zone.rebuild_v5(source.entity).unwrap();
        zone.rebuild_v5(target.entity).unwrap();
        add_overload_arrows(&mut zone, target.entity, &bundle.items);
        (zone, target, npc)
    }

    fn add_overload_arrows(
        zone: &mut ZoneState,
        target: EntityId,
        rules: &crate::domain::zone::Phase4Rules,
    ) {
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&target)
            .unwrap();
        let arrow_weight = i64::from(rules.parts().items[&17].weight);
        let count = u64::try_from(
            player
                .max_load
                .checked_mul(7)
                .unwrap()
                .checked_div(10)
                .unwrap()
                .checked_sub(player.weight)
                .unwrap()
                .checked_div(arrow_weight)
                .unwrap(),
        )
        .unwrap();
        let id = crate::domain::zone::ItemInstanceId::new(900).unwrap();
        player.inventory.insert(
            id,
            crate::domain::zone::ItemInstance {
                creation_origin: None,
                id,
                template_id: 17,
                count,
                location: crate::domain::zone::ItemLocation::Inventory,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: 1,
            },
        );
        zone.rebuild_v5(target).unwrap();
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&target];
        assert_eq!((player.weapon_penalty, player.armor_penalty, player.overload_level), (2, 1, 2));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One lifecycle proves failed and successful admission share no providers.
    fn v6_equipment_survives_death_dead_checkpoint_restore_and_town_resurrection() {
        let (mut zone, target, npc) = equipment_fixture();
        let equipment_before =
            zone.phase4.as_ref().unwrap().canonical.players[&target.entity].clone();
        let living = zone.effective(target.entity).unwrap();
        set_resources(&mut zone, target, 1, 0);
        zone.apply_damage_rolled_v5(
            Tick(1),
            DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target,
                cause: DamageCause::Direct,
                rolled_damage: 50,
            },
            &mut Vec::new(),
            &mut scripted(&[(DamageRollSite::DeathPenalty, 100, 99)], &mut Vec::new()),
        )
        .unwrap();
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&target.entity],
            equipment_before
        );
        assert_eq!(zone.effective(target.entity).unwrap().physical, living.physical);
        assert_eq!(zone.entities[&target.entity].combat.as_ref().unwrap().hp, 0);
        let phase = &zone.phase3.as_ref().unwrap().canonical;
        let checkpoint = phase.checkpoint(target.entity, Tick(1)).unwrap();
        let item_checkpoint = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(target.entity, Tick(1), phase)
            .unwrap();
        let combat = zone.entities[&target.entity].combat.as_ref().unwrap();
        let CombatRole::Player {
            progression: Some(progression),
            class,
            xp,
        } = &combat.role
        else {
            panic!("player");
        };
        let load = PlayerLoad {
            phase4: Some(crate::domain::zone::Phase4PlayerLoad {
                checkpoint: Some(item_checkpoint),
                pending: Vec::new(),
            }),
            phase3: Some(checkpoint),
            progression: Some(progression.clone()),
            checkpoint_revision: Some(1),
            class: class.clone(),
            xp: *xp,
            level: combat.sheet.level(),
            hp: Some(0),
            mp: Some(combat.mp),
            alive: false,
        };
        let sources = load.phase3.as_ref().unwrap().modifier_install_order.len()
            + equipment_before.equipment_sources.len();
        let next_sequence = phase.next_install_sequence;
        let mut invalid = load.clone();
        invalid
            .phase4
            .as_mut()
            .unwrap()
            .checkpoint
            .as_mut()
            .unwrap()
            .rules_hash = [0; 32];
        assert_eq!(zone.validate_paused_load_v5(Some(&invalid)), Err(RejectReason::InvalidLoad));
        let mut corrupt_sources = load.clone();
        corrupt_sources
            .phase4
            .as_mut()
            .unwrap()
            .checkpoint
            .as_mut()
            .unwrap()
            .equipment_install_order
            .pop();
        let spawn = ZoneCommand::SpawnPlayer {
            entity: target.entity,
            name: "Second".into(),
            pos: zone.entities[&target.entity].pos,
            speed: Speed::DEFAULT,
            generation: SessionGeneration(2),
            load: Some(Box::new(load)),
        };
        let mut invalid_spawn = spawn.clone();
        if let ZoneCommand::SpawnPlayer { load, .. } = &mut invalid_spawn {
            *load = Some(Box::new(corrupt_sources));
        }
        let result = zone
            .run_tick(zone.draft(vec![
                ZoneInput::system(ZoneCommand::Despawn {
                    entity: target.entity,
                }),
                ZoneInput::system(invalid_spawn),
                ZoneInput::system(spawn),
            ]))
            .unwrap();
        assert_eq!(result.dispositions.len(), 1);
        assert_eq!(result.dispositions[0].reason, RejectReason::InvalidLoad);
        assert_eq!(
            zone.phase3
                .as_ref()
                .unwrap()
                .canonical
                .next_install_sequence,
            next_sequence
                .checked_add(u64::try_from(sources).unwrap())
                .unwrap()
        );
        let restored = &zone.phase4.as_ref().unwrap().canonical.players[&target.entity];
        assert_eq!(restored.inventory, equipment_before.inventory);
        assert_eq!(
            restored.equipment_sources.values().collect::<Vec<_>>(),
            equipment_before
                .equipment_sources
                .values()
                .collect::<Vec<_>>()
        );
        assert_eq!(zone.effective(target.entity).unwrap().physical, living.physical);
        assert_eq!(zone.entities[&target.entity].combat.as_ref().unwrap().hp, 0);
        let retained = zone.phase4.as_ref().unwrap().canonical.players[&target.entity].clone();
        assert_eq!(zone.entities[&target.entity].generation, SessionGeneration(2));
        let cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        assert_eq!(cold.state_digest(), zone.state_digest());
        zone.respawn_town_v5(Tick(2), target.entity).unwrap();
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.players[&target.entity], retained);
        assert_eq!(zone.effective(target.entity).unwrap().physical, living.physical);
        assert!(zone.entities[&target.entity].combat.as_ref().unwrap().hp > 0);
        assert_eq!(zone.sidecar(target.entity).unwrap().effects.entries().len(), 0);
    }

    #[test]
    fn live_town_respawn_restores_from_corpse_and_emits_final_life_vitals() {
        let (mut zone, _, target, npc) = fixture();
        set_resources(&mut zone, target, 1, 20);
        zone.apply_damage_rolled_v5(
            Tick(1),
            DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target,
                cause: DamageCause::Direct,
                rolled_damage: 10,
            },
            &mut Vec::new(),
            &mut scripted(&[(DamageRollSite::DeathPenalty, 100, 0)], &mut Vec::new()),
        )
        .unwrap();
        zone.run_tick(zone.draft(vec![])).unwrap();
        let mut restored = ZoneState::from_snapshot(zone.snapshot()).unwrap();
        let command = ZoneInput::system(ZoneCommand::Respawn {
            entity: target.entity,
        });
        let draft = zone.draft(vec![command.clone(), command]);
        let actual = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(restored.run_tick(draft).unwrap(), actual);
        assert_eq!(actual.dispositions.len(), 1);
        assert_eq!(actual.dispositions[0].reason, RejectReason::NotDead);
        let c = zone.entities[&target.entity].combat.as_ref().unwrap();
        assert_eq!(c.incarnation, 2);
        assert!(c.hp > 0);
        assert_eq!(c.mp, 0);
        assert_eq!(c.protected_until, Some(Tick(actual.tick.0.checked_add(6000).unwrap())));
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            panic!("player")
        };
        assert_eq!(p.class_state.cp, 20);
        let side = zone.sidecar(target.entity).unwrap();
        assert!(side.player.as_ref().unwrap().last_death.is_none());
        assert_eq!(side.player.as_ref().unwrap().death_penalty_level, 1);
        assert_eq!(side.fractions, ResourceFractions::default());
        assert_eq!(
            actual
                .events
                .iter()
                .filter(|e| matches!(e,
            ZoneEvent::VitalsChanged { entity, incarnation: 2, hp, baseline: None, .. }
            if *entity == target.entity && *hp == c.hp))
                .count(),
            1
        );
        ZoneState::from_snapshot(zone.snapshot()).unwrap();
    }

    #[test]
    fn paused_dead_admission_rebinds_loss_record_without_replaying_death() {
        let (mut zone, _, target, npc) = fixture();
        zone.entities
            .get_mut(&target.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .incarnation = 7;
        let target = zone.binding(target.entity).unwrap();
        set_resources(&mut zone, target, 1, 20);
        zone.apply_damage_rolled_v5(
            Tick(1),
            DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target,
                cause: DamageCause::Direct,
                rolled_damage: 10,
            },
            &mut Vec::new(),
            &mut scripted(&[(DamageRollSite::DeathPenalty, 100, 0)], &mut Vec::new()),
        )
        .unwrap();
        zone.run_tick(zone.draft(vec![])).unwrap();
        let checkpoint = zone
            .snapshot()
            .phase3
            .unwrap()
            .checkpoint(target.entity, Tick(1))
            .unwrap();
        assert_eq!(checkpoint.last_death.as_ref().unwrap().incarnation, 7);
        let c = zone.entities[&target.entity].combat.as_ref().unwrap();
        let CombatRole::Player {
            class,
            xp,
            progression,
        } = &c.role
        else {
            panic!("player")
        };
        let mut load = PlayerLoad::fresh(class.clone());
        load.xp = *xp;
        load.hp = Some(0);
        load.mp = Some(c.mp);
        load.alive = false;
        load.progression = progression.clone();
        load.phase3 = Some(checkpoint.clone());
        let cursor = zone.rng.get_word_pos();
        let applied = zone
            .run_tick(zone.draft(vec![
                ZoneInput::system(ZoneCommand::Despawn {
                    entity: target.entity,
                }),
                ZoneInput::system(ZoneCommand::SpawnPlayer {
                    entity: target.entity,
                    name: "Second".into(),
                    pos: Vec2Fixed::from_tiles(10, 10),
                    speed: Speed::DEFAULT,
                    generation: SessionGeneration(2),
                    load: Some(Box::new(load)),
                }),
            ]))
            .unwrap();
        assert!(applied.dispositions.is_empty());
        assert_eq!(zone.rng.get_word_pos(), cursor);
        let side = zone.sidecar(target.entity).unwrap();
        let death = side.player.as_ref().unwrap().last_death.as_ref().unwrap();
        assert_eq!(death.incarnation, zone.binding(target.entity).unwrap().incarnation);
        assert_eq!(death.pre_death_xp, checkpoint.last_death.as_ref().unwrap().pre_death_xp);
        assert_eq!(
            death.recoverable_death_xp_loss,
            checkpoint
                .last_death
                .as_ref()
                .unwrap()
                .recoverable_death_xp_loss
        );
        assert!(!applied
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::EntityDied { .. })));
        ZoneState::from_snapshot(zone.snapshot()).unwrap();
    }

    type Trace = Vec<(DamageRollSite, TargetBinding, u32, u32, u128)>;

    fn pvp_stat_fixture(zone: &mut ZoneState, id: EntityId) {
        use crate::domain::zone::{
            stat_functions::{FunctionKey, FunctionOp, StatCalculator, StatFunction},
            FinalStats, Scaled,
        };
        // Synthetic, distinguishable multipliers isolate caller selection without promoting
        // deferred skill3552/catalogue rows ahead of the coordinated generated-table update.
        let mut stats = (*zone.effective(id).unwrap()).clone();
        stats.physical = StatSheet::from_final(FinalStats {
            level: 1,
            max_hp: 10_000,
            max_mp: 10_000,
            p_atk: Scaled::from_int(77).unwrap(),
            p_def: Scaled::from_int(76).unwrap(),
            accuracy: Scaled::ZERO,
            evasion: Scaled::ZERO,
            crit_permille: 0,
            attack_speed: Scaled::from_int(300).unwrap(),
            random_damage: 0,
        })
        .unwrap();
        stats.m_atk_q = 64 * Q;
        stats.m_def_q = 91 * Q;
        stats.magic_crit = 0;
        stats.traits.clear();
        stats.calculator = StatCalculator::try_new(
            [
                ("pvpPhysDmg", 2),
                ("pvpPhysSkillsDmg", 3),
                ("pvpMagicalDmg", 5),
            ]
            .into_iter()
            .map(|(stat, factor)| StatFunction {
                key: FunctionKey {
                    stat: stat.into(),
                    priority: 32,
                    install_sequence: 1,
                    handler_ordinal: 0,
                    function_ordinal: 0,
                },
                operation: FunctionOp::Mul(Scaled::from_int(factor).unwrap().raw()),
                conditions: Vec::new(),
            }),
        )
        .unwrap();
        zone.phase3
            .as_mut()
            .unwrap()
            .effective
            .insert(id, Arc::new(stats));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Compare V5/V6 and all physical/magic/static callers with their draw traces.
    fn v6_pvp_damage_callers_select_stats_before_truncation_without_extra_draws() {
        use crate::domain::zone::skill::{RankValue, SkillId};
        use crate::domain::zone::AttackOutcome;
        use DamageRollSite as R;
        let (mut original, target, npc) = equipment_fixture();
        let source = original
            .binding(EntityId::from_uuid(Uuid::from_u128(1)))
            .unwrap();
        for id in [source.entity, target.entity, npc.entity] {
            pvp_stat_fixture(&mut original, id);
        }
        set_resources(&mut original, target, 10_000, 10_000);
        // Formulas.java:696–709: 76*77/76 = 77, then the PvP multiplier, then truncation.
        // Physical :774,799–809: 77*(37+77)/76*3 = 346.5, trunc=346 (not trunc(115.5)*3=345).
        // Magic :829,878–887: 91*7*sqrt(64)/91 = 56; magic uses5, physical/static uses3.
        for legacy in [false, true] {
            let mut baseline = original.clone();
            if legacy {
                baseline.phase4 = None;
            }
            let a = baseline.effective(source.entity).unwrap();
            for stat in ["pvpPhysDmg", "pvpPhysSkillsDmg", "pvpMagicalDmg"] {
                assert_eq!(
                    baseline
                        .item_pvp_damage_bonus(source.entity, npc.entity, &a, stat)
                        .unwrap(),
                    ExactRatio::whole(1)
                );
                assert_eq!(
                    baseline
                        .item_pvp_damage_bonus(npc.entity, target.entity, &a, stat)
                        .unwrap(),
                    ExactRatio::whole(1)
                );
            }
            let mut auto = baseline.clone();
            let mut trace = Vec::new();
            let expected = [(R::AutoHit, 1000, 0), (R::AutoCritical, 1000, 999)];
            assert_eq!(
                auto.auto_damage_v5(source, target, &mut scripted(&expected, &mut trace))
                    .unwrap(),
                (AttackOutcome::Hit, if legacy { 77 } else { 154 })
            );
            assert_eq!(trace.len(), 2);
            for magic in [None, Some(0), Some(1), Some(2)] {
                let mut zone = baseline.clone();
                let mut skill = zone
                    .phase3
                    .as_ref()
                    .unwrap()
                    .canonical
                    .rules
                    .skills()
                    .get(SkillId(if magic.is_some() { 1177 } else { 3 }))
                    .unwrap()
                    .clone();
                if let Some(kind) = magic {
                    skill.is_magic = kind;
                }
                skill.cast_range = RankValue::Scalar(600);
                let mut handler = skill
                    .effects
                    .iter()
                    .find(|handler| {
                        handler.name
                            == if magic.is_some() {
                                "MagicalAttackRange"
                            } else {
                                "PhysicalAttack"
                            }
                    })
                    .unwrap()
                    .clone();
                handler.params.insert(
                    "power".into(),
                    RankValue::Scalar(if magic.is_some() { "7" } else { "37" }.into()),
                );
                handler
                    .params
                    .insert("criticalChance".into(), RankValue::Scalar("0".into()));
                let expected = if magic.is_some() {
                    vec![
                        (R::MagicCritical, 1000, 999),
                        (R::MagicSuccess, 100, 0),
                        (R::MagicVengeance, 100, 99),
                    ]
                } else {
                    vec![(R::PhysicalEvasion, 100, 99)]
                };
                let mut trace = Vec::new();
                let row = if magic.is_some() {
                    zone.magic_handler_rolled_v5(
                        Tick(1),
                        source,
                        target,
                        &skill,
                        1,
                        &handler,
                        0,
                        &mut Vec::new(),
                        &mut scripted(&expected, &mut trace),
                    )
                } else {
                    zone.physical_handler_rolled_v5(
                        Tick(1),
                        source,
                        target,
                        &skill,
                        1,
                        &handler,
                        0,
                        &mut Vec::new(),
                        &mut scripted(&expected, &mut trace),
                    )
                }
                .unwrap();
                let expected_damage = match (legacy, magic) {
                    (true, None) => 115,
                    (false, None) => 346,
                    (true, Some(_)) => 56,
                    (false, Some(1)) => 280,
                    (false, Some(_)) => 168,
                };
                assert_eq!(row.rolled, expected_damage, "legacy={legacy}, magic={magic:?}");
                assert_eq!(trace.len(), expected.len());
            }
        }
    }

    fn scripted<'a>(
        expected: &'a [(DamageRollSite, u32, u32)],
        trace: &'a mut Trace,
    ) -> impl FnMut(&mut ZoneState, DamageRollSite, TargetBinding, u32) -> Result<u32, StatError> + 'a
    {
        let mut values = expected.iter();
        move |zone, site, target, bound| {
            let &(expected_site, expected_bound, value) = values.next().expect("unexpected draw");
            assert_eq!((site, bound), (expected_site, expected_bound));
            assert!(value < bound);
            zone.roll_below(bound);
            trace.push((site, target, bound, value, zone.rng.get_word_pos()));
            Ok(value)
        }
    }
    fn start_cast(zone: &mut ZoneState, caster: TargetBinding, target: TargetBinding) {
        let c = zone
            .entities
            .get_mut(&caster.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        else {
            panic!("player")
        };
        p.class_state.learned_skills = vec![crate::domain::subclass::LearnedSkill {
            key: "l2.skill.1177".into(),
            level: 1,
        }];
        let tick = zone
            .run_tick(zone.draft(vec![ZoneInput {
                source: CommandSource::Session {
                    entity: caster.entity,
                    generation: SessionGeneration(1),
                },
                seq: Some(1),
                command: ZoneCommand::CastSkill {
                    entity: caster.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1177),
                    target: Some(target.entity),
                },
            }]))
            .unwrap();
        assert!(tick.dispositions.is_empty());
        assert!(matches!(zone.sidecar(caster.entity).unwrap().cast, CastState::Casting(_)));
    }
    fn add_stun(zone: &mut ZoneState, target: TargetBinding) {
        let rules = Arc::clone(&zone.phase3.as_ref().unwrap().canonical.rules);
        let mut stun = crate::domain::zone::phase3_entity::tests::source_effect(&rules, 1, 100);
        stun.target_incarnation = target.incarnation;
        let side = zone.sidecar_mut(target.entity).unwrap();
        side.effects = EffectList::from_entries([(stun.id, stun)].into()).unwrap();
        side.modifier_install_order
            .insert(1, RestoredModifierSource::Effect(effects::EffectId(1)));
        zone.rebuild_v5(target.entity).unwrap();
    }

    #[test]
    fn injected_magic_tail_interrupts_before_vengeance_and_recipient_stun_cp() {
        use DamageRollSite as R;
        let (original, source, target, npc) = fixture();
        for outside_window in [false, true] {
            let mut zone = original.clone();
            start_cast(&mut zone, target, npc);
            // A unit-level recipient status fixture preserves the existing cast deliberately
            // to exercise source ordering; normal stun installation cancels it immediately.
            add_stun(&mut zone, target);
            set_resources(&mut zone, target, 50, 1000);
            let CastState::Casting(cast) = zone.sidecar(target.entity).unwrap().cast.clone() else {
                panic!("cast")
            };
            let tick = if outside_window {
                cast.interrupt_until
            } else {
                cast.start_at
            };
            let skill = zone
                .phase3
                .as_ref()
                .unwrap()
                .canonical
                .rules
                .skills()
                .get(crate::domain::zone::skill::SkillId(1177))
                .unwrap()
                .clone();
            let handler = skill
                .effects
                .iter()
                .find(|h| h.name == "MagicalAttackRange")
                .unwrap();
            let radius = zone
                .effective(source.entity)
                .unwrap()
                .physical
                .random_damage();
            let expected = [
                (R::MagicCritical, 1000, 999),
                (R::MagicSuccess, 100, 0),
                (R::Spread, radius.saturating_mul(2).saturating_add(1), radius),
                (R::AttackBreak, 100, 0),
                (R::MagicVengeance, 100, 99),
                (R::StunRemoval, 10, 0),
            ];
            let mut trace = Vec::new();
            let before_cursor = zone.rng.get_word_pos();
            let mut events = Vec::new();
            let row = zone
                .magic_handler_rolled_v5(
                    tick,
                    source,
                    target,
                    &skill,
                    1,
                    handler,
                    0,
                    &mut events,
                    &mut scripted(&expected, &mut trace),
                )
                .unwrap();
            assert_eq!(trace.len(), expected.len());
            assert!(trace.iter().all(|r| r.1 == target));
            assert!(trace.windows(2).all(|w| w[0].4 < w[1].4));
            assert!(trace[0].4 > before_cursor);
            assert!(row.cp_absorbed > 0);
            assert_eq!(row.hp_removed, 0);
            let canceled = events.iter().position(|e| {
                matches!(
                    e,
                    ZoneEvent::SkillCastCancelled {
                        reason: CastCancelReason::Damage,
                        ..
                    }
                )
            });
            let removed = events
                .iter()
                .position(|e| matches!(e, ZoneEvent::EffectRemoved { .. }))
                .unwrap();
            assert_eq!(canceled.is_some(), !outside_window);
            if let Some(canceled) = canceled {
                assert!(canceled < removed);
            }
            assert_eq!(
                matches!(zone.sidecar(target.entity).unwrap().cast, CastState::Casting(_)),
                outside_window
            );
        }
    }

    #[test]
    fn injected_magic_failure_second_roll_is_player_only_and_idle_skips_break() {
        use DamageRollSite as R;
        let (original, player, target, npc) = fixture();
        for source in [player, npc] {
            let mut zone = original.clone();
            set_resources(&mut zone, target, 50, 1000);
            let skill = zone
                .phase3
                .as_ref()
                .unwrap()
                .canonical
                .rules
                .skills()
                .get(crate::domain::zone::skill::SkillId(1177))
                .unwrap()
                .clone();
            let handler = skill
                .effects
                .iter()
                .find(|h| h.name == "MagicalAttackRange")
                .unwrap();
            let radius = zone
                .effective(source.entity)
                .unwrap()
                .physical
                .random_damage();
            let mut expected = vec![(R::MagicCritical, 1000, 999), (R::MagicSuccess, 100, 99)];
            if source == player {
                expected.push((R::MagicFailure, 100, 0));
            }
            if radius > 0 {
                expected.push((R::Spread, radius.saturating_mul(2).saturating_add(1), radius));
            }
            expected.push((R::MagicVengeance, 100, 99));
            let mut trace = Vec::new();
            let row = zone
                .magic_handler_rolled_v5(
                    Tick(1),
                    source,
                    target,
                    &skill,
                    1,
                    handler,
                    0,
                    &mut Vec::new(),
                    &mut scripted(&expected, &mut trace),
                )
                .unwrap();
            assert_eq!(trace.len(), expected.len());
            assert_eq!(
                row.outcome,
                if source == player {
                    SkillOutcome::Half
                } else {
                    SkillOutcome::Resist
                }
            );
            assert_eq!(row.cp_absorbed > 0, source == player);
        }
    }

    #[test]
    fn injected_physical_recipient_stun_precedes_zero_chance_vengeance_without_attack_break() {
        use DamageRollSite as R;
        let (mut zone, source, target, npc) = fixture();
        start_cast(&mut zone, target, npc);
        add_stun(&mut zone, target);
        set_resources(&mut zone, target, 5000, 1000);
        let skill = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(crate::domain::zone::skill::SkillId(3))
            .unwrap()
            .clone();
        let handler = skill
            .effects
            .iter()
            .find(|h| h.name == "PhysicalAttack")
            .unwrap();
        let radius = zone
            .effective(source.entity)
            .unwrap()
            .physical
            .random_damage();
        let chance: i32 = handler
            .params
            .get("criticalChance")
            .and_then(|p| p.at(1, skill.max_level))
            .map_or(0, |value| value.parse().unwrap());
        let mut expected = vec![(R::PhysicalEvasion, 100, 99)];
        if chance > 0 {
            expected.push((R::PhysicalCritical, 100_000_000, 99_999_999));
        }
        expected.extend([
            (R::Spread, radius.saturating_mul(2).saturating_add(1), radius),
            (R::StunRemoval, 10, 0),
            (R::PhysicalVengeance, 100, 99),
        ]);
        let mut trace = Vec::new();
        let row = zone
            .physical_handler_rolled_v5(
                Tick(2),
                source,
                target,
                &skill,
                1,
                handler,
                0,
                &mut Vec::new(),
                &mut scripted(&expected, &mut trace),
            )
            .unwrap();
        assert_eq!(trace.len(), expected.len());
        assert!(row.cp_absorbed > 0);
        assert!(matches!(zone.sidecar(target.entity).unwrap().cast, CastState::Casting(_)));
    }

    #[test]
    fn injected_periodic_has_no_stun_break_or_reflection_even_at_hp_one() {
        let (mut zone, source, target, _) = fixture();
        add_stun(&mut zone, target);
        set_resources(&mut zone, target, 1, 20);
        let mut trace = Vec::new();
        let result = zone
            .apply_damage_rolled_v5(
                Tick(1),
                DamageRequest {
                    source: zone.damage_source_v5(source).unwrap(),
                    target,
                    cause: DamageCause::Periodic,
                    rolled_damage: 1000,
                },
                &mut Vec::new(),
                &mut scripted(&[], &mut trace),
            )
            .unwrap();
        assert_eq!((result.cp_absorbed, result.hp_removed), (0, 0));
        assert_eq!(zone.sidecar(target.entity).unwrap().effects.entries().len(), 1);
        assert!(trace.is_empty());
    }
    #[test]
    fn injected_attack_break_draw_survives_physical_kind_and_cutoff_refusals() {
        use DamageRollSite as R;
        let (original, _, target, npc) = fixture();
        for (physical, expired, roll, cancels) in [
            (false, false, 0, true),
            (false, false, 99, false),
            (true, false, 0, false),
            (false, true, 0, false),
        ] {
            let mut zone = original.clone();
            start_cast(&mut zone, target, npc);
            let CastState::Casting(cast) = &mut zone.sidecar_mut(target.entity).unwrap().cast
            else {
                panic!("cast")
            };
            if physical {
                cast.skill.id = crate::domain::zone::skill::SkillId(3);
            }
            let tick = if expired {
                cast.interrupt_until
            } else {
                cast.start_at
            };
            let expected = [(R::AttackBreak, 100, roll)];
            let mut trace = Vec::new();
            let mut events = Vec::new();
            zone.attack_break_v5(
                tick,
                target,
                1,
                &mut events,
                &mut scripted(&expected, &mut trace),
            )
            .unwrap();
            assert_eq!(trace.len(), 1);
            assert_eq!(
                matches!(zone.sidecar(target.entity).unwrap().cast, CastState::Idle),
                cancels
            );
            assert_eq!(!events.is_empty(), cancels);
        }
    }

    #[test]
    fn injected_full_magic_resist_keeps_zero_whole_damage_tail_and_breaks_sleep() {
        use DamageRollSite as R;
        let (mut zone, source, target, _) = fixture();
        let rules = Arc::clone(&zone.phase3.as_ref().unwrap().canonical.rules);
        let mut sleep = crate::domain::zone::phase3_entity::tests::source_effect(&rules, 1, 1069);
        sleep.target_incarnation = target.incarnation;
        let side = zone.sidecar_mut(target.entity).unwrap();
        side.effects = EffectList::from_entries([(sleep.id, sleep)].into()).unwrap();
        side.modifier_install_order
            .insert(1, RestoredModifierSource::Effect(effects::EffectId(1)));
        set_resources(&mut zone, target, 50, 20);
        let skill = rules
            .skills()
            .get(crate::domain::zone::skill::SkillId(1177))
            .unwrap();
        let handler = skill
            .effects
            .iter()
            .find(|h| h.name == "MagicalAttackRange")
            .unwrap();
        let radius = zone
            .effective(source.entity)
            .unwrap()
            .physical
            .random_damage();
        assert!(radius > 0);
        let expected = [
            (R::MagicCritical, 1000, 999),
            (R::MagicSuccess, 100, 99),
            (R::MagicFailure, 100, 99),
            (R::Spread, radius.saturating_mul(2).saturating_add(1), 0),
            (R::MagicVengeance, 100, 99),
        ];
        let mut trace = Vec::new();
        let mut events = Vec::new();
        let row = zone
            .magic_handler_rolled_v5(
                Tick(1),
                source,
                target,
                skill,
                1,
                handler,
                0,
                &mut events,
                &mut scripted(&expected, &mut trace),
            )
            .unwrap();
        assert_eq!(trace.len(), expected.len());
        assert_eq!(row.outcome, SkillOutcome::Resist);
        assert_eq!((row.rolled, row.hp_removed, row.cp_absorbed), (0, 0, 0));
        assert!(zone
            .sidecar(target.entity)
            .unwrap()
            .effects
            .entries()
            .is_empty());
        assert!(events.iter().any(|e| matches!(
            e,
            ZoneEvent::EffectRemoved {
                reason: effects::EffectRemoveReason::Damage,
                ..
            }
        )));
    }
    #[test]
    fn cold_hostile_player_cast_is_accepted_by_completed_runtime_policy() {
        let (mut zone, source, target, npc) = fixture();
        start_cast(&mut zone, source, npc);
        let mut snapshot = zone.snapshot();
        assert!(ZoneState::from_snapshot(snapshot.clone()).is_ok());
        let CastState::Casting(cast) = &mut snapshot
            .phase3
            .as_mut()
            .unwrap()
            .entities
            .get_mut(&source.entity)
            .unwrap()
            .cast
        else {
            panic!("cast")
        };
        cast.target = Some(target);
        cast.targets = vec![target];
        assert!(ZoneState::from_snapshot(snapshot).is_ok());
    }
    #[test]
    fn zero_damage_gets_attacked_hate_and_the_source_zero_aggro_fallback() {
        let (mut zone, player, _, npc) = fixture();
        let mut events = Vec::new();
        let before_hp = zone.entities[&npc.entity].combat.as_ref().unwrap().hp;
        let result = zone
            .apply_damage_v5(
                Tick(1),
                DamageRequest {
                    source: zone.damage_source_v5(player).unwrap(),
                    target: npc,
                    cause: DamageCause::Periodic,
                    rolled_damage: 0,
                },
                &mut events,
            )
            .unwrap();
        assert_eq!((result.hp_removed, result.hp_after), (0, before_hp));
        assert_eq!(
            zone.hate[&npc.entity].get(player.entity).unwrap(),
            crate::domain::zone::HateEntry { hate: 2, damage: 0 }
        );
        let hates: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let ZoneEvent::HateChanged { hate, .. } = event {
                    Some(*hate)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(hates, [1, 2]);
    }
    fn review_input(entity: EntityId, seq: u64, command: ZoneCommand) -> ZoneInput {
        ZoneInput::session(entity, SessionGeneration(1), u32::try_from(seq).unwrap(), command)
    }
    fn review_learn(zone: &mut ZoneState, entity: EntityId, skill: u32) {
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut zone
            .entities
            .get_mut(&entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            panic!("player")
        };
        p.class_state
            .learned_skills
            .push(crate::domain::subclass::LearnedSkill {
                key: format!("l2.skill.{skill}"),
                level: 1,
            });
        p.class_state
            .learned_skills
            .sort_by(|a, b| a.key.cmp(&b.key));
    }
    fn review_swing(
        zone: &mut ZoneState,
        source: TargetBinding,
        target: TargetBinding,
        impact: Tick,
    ) {
        let entity = zone.entities.get_mut(&source.entity).unwrap();
        entity.targeting.target = Some(target.entity);
        let c = entity.combat.as_mut().unwrap();
        c.auto_attack = true;
        c.ready_at = Tick(impact.0.checked_add(50).unwrap());
        c.swing = Some(crate::domain::zone::Swing {
            target: target.entity,
            target_incarnation: target.incarnation,
            start: Tick(0),
            impact,
            ready: c.ready_at,
        });
    }
    fn review_complete(zone: &mut ZoneState, caster: TargetBinding) -> AppliedTick {
        let CastState::Casting(cast) = zone.sidecar(caster.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        while zone.next_tick < cast.complete_at {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        zone.run_tick(zone.draft(Vec::new())).unwrap()
    }

    #[test]
    fn review_nonlethal_pvp_cast_snapshot_envelope_restores_and_continues() {
        use crate::application::replay_log::{decode_snapshot, encode_snapshot};
        let (mut zone, source, target, _) = fixture();
        start_cast(&mut zone, source, target);
        let tick = review_complete(&mut zone, source);
        assert!(tick
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::SkillCastResult { .. })));
        assert!(!zone.entities[&target.entity].targeting.dead);
        assert!(zone
            .hate
            .keys()
            .all(|id| zone.entities[id].kind == EntityKind::Npc));
        assert!(!tick
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::HateChanged { npc, .. } if *npc == target.entity)));
        let snapshot = decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap();
        let mut cold = ZoneState::from_snapshot(snapshot).unwrap();
        for _ in 0..35 {
            assert_eq!(
                zone.run_tick(zone.draft(Vec::new())).unwrap(),
                cold.run_tick(cold.draft(Vec::new())).unwrap()
            );
        }
    }

    #[test]
    fn review_effect_point_eligibility_and_pinned_positive_existing_row_branch() {
        let (mut zone, source, target, npc) = fixture();
        let tick = zone.next_tick;
        let mut events = Vec::new();
        for point in [-30, 30] {
            zone.effect_point_hate_v5(tick, source, target, point, &mut events);
            zone.effect_point_hate_v5(tick, npc, target, point, &mut events);
        }
        zone.effect_point_hate_v5(tick, source, npc, 30, &mut events);
        assert!(zone.hate.is_empty());
        assert!(events.is_empty());
        zone.effect_point_hate_v5(tick, source, npc, -30, &mut events);
        assert_eq!(zone.hate[&npc.entity].get(source.entity).unwrap().hate, 30);
        // Despite its name, pinned reduceHate adds positive effectPoint to an existing row.
        zone.effect_point_hate_v5(tick, source, npc, 20, &mut events);
        assert_eq!(zone.hate[&npc.entity].get(source.entity).unwrap().hate, 50);
        assert!(zone
            .hate
            .keys()
            .all(|id| zone.entities[id].kind == EntityKind::Npc));
    }

    #[test]
    fn review_receive_order_cast_attack_and_later_stop_selection_move() {
        for cast_first in [true, false] {
            let (mut zone, source, target, _) = fixture();
            review_learn(&mut zone, source.entity, 1177);
            let mut commands = vec![review_input(
                source.entity,
                1,
                ZoneCommand::SetTarget {
                    entity: source.entity,
                    target: Some(target.entity),
                },
            )];
            let cast = ZoneCommand::CastSkill {
                entity: source.entity,
                skill_id: crate::domain::zone::skill::SkillId(1177),
                target: None,
            };
            let attack = ZoneCommand::Attack {
                entity: source.entity,
            };
            let actions = if cast_first {
                [cast, attack]
            } else {
                [attack, cast]
            };
            commands.extend(
                actions
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| review_input(source.entity, u64::try_from(i).unwrap() + 2, c)),
            );
            let cursor = zone.rng.get_word_pos();
            let tick = zone.run_tick(zone.draft(commands)).unwrap();
            assert_eq!(tick.dispositions.len(), usize::from(cast_first));
            if cast_first {
                assert_eq!(tick.dispositions[0].reason, RejectReason::AlreadyCasting);
                assert_eq!(tick.dispositions[0].seq, Some(3));
            }
            assert!(matches!(zone.sidecar(source.entity).unwrap().cast, CastState::Casting(_)));
            assert!(!tick.events.iter().any(|e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == source.entity)));
            assert_eq!(zone.rng.get_word_pos(), cursor);
        }
        for cancel in 0..3 {
            let (mut zone, source, target, _) = fixture();
            let command = match cancel {
                0 => ZoneCommand::StopAttack {
                    entity: source.entity,
                },
                1 => ZoneCommand::SetTarget {
                    entity: source.entity,
                    target: None,
                },
                _ => ZoneCommand::MoveTo {
                    entity: source.entity,
                    dest: Vec2Fixed::from_tiles(15, 10),
                },
            };
            let tick = zone
                .run_tick(zone.draft(vec![
                    review_input(
                        source.entity,
                        1,
                        ZoneCommand::SetTarget {
                            entity: source.entity,
                            target: Some(target.entity),
                        },
                    ),
                    review_input(
                        source.entity,
                        2,
                        ZoneCommand::Attack {
                            entity: source.entity,
                        },
                    ),
                    review_input(source.entity, 3, command),
                ]))
                .unwrap();
            assert!(tick.dispositions.is_empty());
            assert!(
                !zone.entities[&source.entity]
                    .combat
                    .as_ref()
                    .unwrap()
                    .auto_attack
            );
        }
    }

    #[test]
    fn review_attack_guard_runs_after_same_tick_stun_expiry() {
        let (mut zone, source, target, _) = fixture();
        add_stun(&mut zone, source);
        assert!(zone
            .effective(source.entity)
            .unwrap()
            .flags
            .intersects(effects::StatusFlags::STUN));
        zone.entities
            .get_mut(&source.entity)
            .unwrap()
            .targeting
            .target = Some(target.entity);
        let tick = zone
            .run_tick(zone.draft(vec![review_input(
                source.entity,
                1,
                ZoneCommand::Attack {
                    entity: source.entity,
                },
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let removed = tick
            .events
            .iter()
            .position(|e| matches!(e, ZoneEvent::EffectRemoved { .. }))
            .unwrap();
        let started = tick
            .events
            .iter()
            .position(|e| matches!(e, ZoneEvent::AttackStarted { .. }))
            .unwrap();
        assert!(removed < started);
    }

    #[test]
    fn review_cast_lethal_and_stun_complete_before_due_swing_without_auto_rng() {
        for stun in [false, true] {
            let (mut zone, source, target, npc) = fixture();
            // Isolate a single effect target; ordinary gameplay target collection still runs.
            zone.entities
                .get_mut(&npc.entity)
                .unwrap()
                .targeting
                .attackable = false;
            let skill = if stun { 1072 } else { 1177 };
            review_learn(&mut zone, source.entity, skill);
            let start = zone
                .run_tick(zone.draft(vec![review_input(
                    source.entity,
                    1,
                    ZoneCommand::CastSkill {
                        entity: source.entity,
                        skill_id: crate::domain::zone::skill::SkillId(skill),
                        target: Some(target.entity),
                    },
                )]))
                .unwrap();
            assert!(start.dispositions.is_empty(), "{:?}", start.dispositions);
            let CastState::Casting(cast) = zone.sidecar(source.entity).unwrap().cast.clone() else {
                panic!("cast")
            };
            while zone.next_tick < cast.complete_at {
                zone.run_tick(zone.draft(Vec::new())).unwrap();
            }
            if !stun {
                set_resources(&mut zone, target, 1, 0);
            }
            review_swing(&mut zone, target, source, cast.complete_at);
            // Pin the seeded success branch independently of the extra due swing.
            let mut control = zone.clone();
            control
                .entities
                .get_mut(&target.entity)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .swing = None;
            control
                .entities
                .get_mut(&target.entity)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .auto_attack = false;
            let before = zone.rng.get_word_pos();
            let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
            control.run_tick(control.draft(Vec::new())).unwrap();
            if stun {
                assert!(zone
                    .effective(target.entity)
                    .unwrap()
                    .flags
                    .intersects(effects::StatusFlags::STUN));
            } else {
                assert!(zone.entities[&target.entity].targeting.dead);
            }
            assert!(zone.rng.get_word_pos() > before);
            assert_eq!(zone.rng.get_word_pos(), control.rng.get_word_pos());
            assert!(!tick.events.iter().any(|e| matches!(e, ZoneEvent::AttackResult { attacker, .. } if *attacker == target.entity)));
            assert!(tick.events.iter().any(|e| matches!(e, ZoneEvent::AttackCancelled { attacker, .. } if *attacker == target.entity)));
        }
    }

    #[test]
    fn review_ai_acquisition_waits_until_next_tick_to_start_swing() {
        let (mut zone, player, _, npc) = fixture();
        let now = zone.next_tick;
        zone.add_hate(
            now,
            npc.entity,
            player.entity,
            crate::domain::zone::HateEntry { hate: 1, damage: 0 },
            &mut Vec::new(),
        );
        let cursor = zone.rng.get_word_pos();
        let acquired = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(zone.entities[&npc.entity].targeting.target, Some(player.entity));
        assert!(!acquired.events.iter().any(
            |e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == npc.entity)
        ));
        assert_eq!(zone.rng.get_word_pos(), cursor);
        let next = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert!(next.events.iter().any(
            |e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == npc.entity)
        ));
        assert_eq!(zone.rng.get_word_pos(), cursor);
    }

    #[test]
    fn review_simultaneous_lethal_swings_use_id_not_admission_order_and_cold_replay() {
        let (mut zone, first, second, _) = fixture_with_order(true);
        assert!(
            zone.sidecar(first.entity).unwrap().ordinal
                > zone.sidecar(second.entity).unwrap().ordinal
        );
        for (a, b) in [(first, second), (second, first)] {
            set_resources(&mut zone, a, 1, 0);
            let now = zone.next_tick;
            review_swing(&mut zone, a, b, now);
        }
        // Fix a guaranteed-hit seeded cursor (hit minimum 200); no injected damage path.
        while zone.clone().roll_below(1000) > 200 {
            zone.roll_below(1000);
        }
        let mut cold = ZoneState::from_snapshot(zone.snapshot()).unwrap();
        let before = zone.rng.get_word_pos();
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(tick, cold.run_tick(cold.draft(Vec::new())).unwrap());
        let attackers: Vec<_> = tick
            .events
            .iter()
            .filter_map(|e| {
                if let ZoneEvent::AttackResult { attacker, .. } = e {
                    Some(*attacker)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(attackers, vec![first.entity]);
        assert!(zone.entities[&second.entity].targeting.dead);
        assert!(!zone.entities[&first.entity].targeting.dead);
        assert_eq!(zone.rng.get_word_pos() - before, 3); // hit, crit, spread; neutral PvP death has no penalty roll
        for _ in 0..10 {
            assert_eq!(
                zone.run_tick(zone.draft(Vec::new())).unwrap(),
                cold.run_tick(cold.draft(Vec::new())).unwrap()
            );
        }
    }

    #[test]
    fn review_accept_resurrection_is_staged_behind_town_lifecycle_in_both_orders() {
        for town_first in [false, true] {
            let (mut zone, caster, target) = resurrection_fixture();
            zone.resurrect_offer_v5(
                zone.next_tick,
                caster,
                target,
                crate::domain::zone::skill::SkillRef {
                    id: crate::domain::zone::skill::SkillId(1016),
                    rank: 2,
                },
                &mut Vec::new(),
            )
            .unwrap();
            let id = zone
                .sidecar(target.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .resurrect_offer
                .as_ref()
                .unwrap()
                .id;
            let accept = ZoneCommand::AcceptResurrect {
                entity: target.entity,
                offer_id: id,
                accept: true,
            };
            let town = ZoneCommand::Respawn {
                entity: target.entity,
            };
            let actions = if town_first {
                [town, accept]
            } else {
                [accept, town]
            };
            let commands = actions
                .into_iter()
                .enumerate()
                .map(|(i, c)| review_input(target.entity, u64::try_from(i).unwrap() + 1, c))
                .collect();
            let tick = zone.run_tick(zone.draft(commands)).unwrap();
            assert_eq!(tick.dispositions.len(), 1);
            assert_eq!(tick.dispositions[0].reason, RejectReason::NoResurrectOffer);
            let c = zone.entities[&target.entity].combat.as_ref().unwrap();
            assert_eq!(c.incarnation, target.incarnation + 1);
            assert!(c.protected_until.is_some());
            assert_eq!(
                tick.events
                    .iter()
                    .filter(|e| matches!(e, ZoneEvent::EntityRespawned { .. }))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn review_cannot_accept_offer_created_later_in_the_same_tick() {
        let (mut zone, caster, target) = resurrection_fixture();
        let started = zone
            .run_tick(zone.draft(vec![review_input(
                caster.entity,
                1,
                ZoneCommand::CastSkill {
                    entity: caster.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1016),
                    target: Some(target.entity),
                },
            )]))
            .unwrap();
        assert!(started.dispositions.is_empty());
        let CastState::Casting(cast) = zone.sidecar(caster.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        while zone.next_tick < cast.complete_at {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        let offer_id =
            crate::domain::zone::OfferId(zone.phase3.as_ref().unwrap().canonical.next_offer_id);
        let tick = zone
            .run_tick(zone.draft(vec![review_input(
                target.entity,
                2,
                ZoneCommand::AcceptResurrect {
                    entity: target.entity,
                    offer_id,
                    accept: true,
                },
            )]))
            .unwrap();
        assert_eq!(tick.dispositions[0].reason, RejectReason::NoResurrectOffer);
        assert!(zone.entities[&target.entity].targeting.dead);
        assert_eq!(
            zone.sidecar(target.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .resurrect_offer
                .as_ref()
                .unwrap()
                .id,
            offer_id
        );
    }

    #[test]
    fn review_missed_auto_refreshes_flag_without_recipient_draws_and_starts_refresh_again() {
        use DamageRollSite as R;
        let (mut zone, source, target, _) = fixture();
        let now = zone.next_tick;
        review_swing(&mut zone, source, target, now);
        let swing = zone.entities[&source.entity]
            .combat
            .as_ref()
            .unwrap()
            .swing
            .unwrap();
        let mut trace = Vec::new();
        let mut events = Vec::new();
        zone.land_auto_v5(
            now,
            source.entity,
            swing,
            &mut events,
            &mut scripted(&[(R::AutoHit, 1000, 999)], &mut trace),
        )
        .unwrap();
        assert_eq!(trace.len(), 1);
        assert!(events.iter().any(|e| matches!(
            e,
            ZoneEvent::AttackResult {
                outcome: crate::domain::zone::AttackOutcome::Miss,
                ..
            }
        )));
        assert_eq!(
            zone.sidecar(source.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .pvp
                .flag_until,
            Some(Tick(now.0 + 1200))
        );
        // A flagged victim gives the shorter refresh, which may not shorten an old deadline.
        zone.sidecar_mut(target.entity)
            .unwrap()
            .player
            .as_mut()
            .unwrap()
            .pvp
            .flag_until = Some(Tick(5000));
        zone.entities
            .get_mut(&source.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .ready_at = now;
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(
            zone.sidecar(source.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .pvp
                .flag_until,
            Some(Tick(now.0 + 1200))
        );
        // Repeat swing start against a now-neutral target extends without a new Attack command.
        zone.sidecar_mut(target.entity)
            .unwrap()
            .player
            .as_mut()
            .unwrap()
            .pvp
            .flag_until = None;
        let c = zone
            .entities
            .get_mut(&source.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        c.swing = None;
        c.ready_at = zone.next_tick;
        let later = zone.next_tick;
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert!(tick.events.iter().any(
            |e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker==source.entity)
        ));
        assert_eq!(
            zone.sidecar(source.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .pvp
                .flag_until,
            Some(Tick(later.0 + 1200))
        );
    }

    #[test]
    fn review_ranged_death_omits_unseen_killer_but_keeps_private_fact() {
        use crate::interface::{grpc::pb, zone_mapping::observer_output_to_pb};
        let (mut zone, source, target, _) = fixture();
        let observer = EntityId::from_uuid(Uuid::from_u128(3));
        let mut load = PlayerLoad::fresh("human_mystic");
        load.progression = Some(Box::new(crate::domain::zone::PlayerProgression {
            identity: CharacterIdentity {
                account_id: AccountId::from_uuid(Uuid::from_u128(3)),
                race: Race::Human,
                base_class_id: ClassId(10),
                appearance: CharacterAppearance::default(),
            },
            class_state: ClassState::new(ClassId(10)),
            max_cp: 0,
        }));
        for (id, pos) in [
            (source.entity, Vec2Fixed::from_tiles(63, 10)),
            (target.entity, Vec2Fixed::from_tiles(64, 10)),
        ] {
            let old = zone.entities[&id].pos;
            zone.entities.get_mut(&id).unwrap().pos = pos;
            zone.aoi.relocate(id, old, pos);
        }
        let admission = zone
            .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: observer,
                name: "Viewer".into(),
                pos: Vec2Fixed::from_tiles(96, 10),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load)),
            })]))
            .unwrap();
        assert!(admission.outputs[&observer].iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity,.. }) if *entity==target.entity)));
        assert!(!admission.outputs[&observer].iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity,.. }) if *entity==source.entity)));
        start_cast(&mut zone, source, target);
        let CastState::Casting(cast) = zone.sidecar(source.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        while zone.next_tick < cast.complete_at {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        set_resources(&mut zone, target, 1, 0);
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert!(tick.events.iter().any(|e| matches!(e, ZoneEvent::EntityDied { entity,killer:Some(k),.. } if *entity==target.entity && *k==source.entity)));
        let death=tick.outputs[&observer].iter().find(|o| matches!(o,ObserverOutput::Event(ZoneEvent::EntityDied { entity,.. }) if *entity==target.entity)).unwrap();
        assert!(matches!(
            death,
            ObserverOutput::Event(ZoneEvent::EntityDied { killer: None, .. })
        ));
        let messages = observer_output_to_pb(death, tick.server_time_ms);
        let pb = &messages[0];
        assert!(
            matches!(&pb.payload,Some(pb::server_message::Payload::Event(pb::WorldEvent { event:Some(pb::world_event::Event::EntityDied(pb::EntityDied { killer,.. })) })) if killer.is_empty())
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Both real impact callers share the same flag-change boundary.
    fn review_resisted_debuff_and_evaded_physical_impact_refresh_hostility() {
        use crate::domain::zone::stat_functions::{
            FunctionKey, FunctionOp, StatCalculator, StatFunction,
        };
        for physical in [false, true] {
            let (mut zone, source, target, npc) = fixture();
            zone.entities
                .get_mut(&npc.entity)
                .unwrap()
                .targeting
                .attackable = false;
            let skill = if physical { 3 } else { 1072 };
            review_learn(&mut zone, source.entity, skill);
            zone.sidecar_mut(target.entity)
                .unwrap()
                .player
                .as_mut()
                .unwrap()
                .pvp
                .flag_until = Some(Tick(5000));
            let start = zone
                .run_tick(zone.draft(vec![review_input(
                    source.entity,
                    1,
                    ZoneCommand::CastSkill {
                        entity: source.entity,
                        skill_id: crate::domain::zone::skill::SkillId(skill),
                        target: Some(target.entity),
                    },
                )]))
                .unwrap();
            assert!(start.dispositions.is_empty(), "{:?}", start.dispositions);
            assert_eq!(
                zone.sidecar(source.entity)
                    .unwrap()
                    .player
                    .as_ref()
                    .unwrap()
                    .pvp
                    .flag_until,
                Some(Tick(start.tick.0 + 600))
            );
            let CastState::Casting(cast) = zone.sidecar(source.entity).unwrap().cast.clone() else {
                panic!("cast")
            };
            while zone.next_tick < cast.complete_at {
                zone.run_tick(zone.draft(Vec::new())).unwrap();
            }
            zone.sidecar_mut(target.entity)
                .unwrap()
                .player
                .as_mut()
                .unwrap()
                .pvp
                .flag_until = None;
            if physical {
                // Explicit derived-stat fixture uses Ultimate Evasion rank-one pSkillEvas=25
                // (skills/111.toml); production dispatch and seeded RNG are unchanged.
                let mut stats = (*zone.effective(target.entity).unwrap()).clone();
                stats.calculator = StatCalculator::try_new([StatFunction {
                    key: FunctionKey {
                        stat: "pSkillEvas".into(),
                        priority: 0,
                        install_sequence: 0,
                        handler_ordinal: 0,
                        function_ordinal: 0,
                    },
                    operation: FunctionOp::Set(25 * Q),
                    conditions: Vec::new(),
                }])
                .unwrap();
                zone.phase3
                    .as_mut()
                    .unwrap()
                    .effective
                    .insert(target.entity, Arc::new(stats));
                while zone.clone().roll_below(100) >= 25 {
                    zone.roll_below(100);
                }
            } else {
                loop {
                    let mut preview = zone.clone();
                    preview.roll_below(100);
                    if preview.roll_below(100) >= 90 {
                        break;
                    }
                    zone.roll_below(100);
                }
            }
            let hp = zone.entities[&target.entity].combat.as_ref().unwrap().hp;
            let cursor = zone.rng.get_word_pos();
            let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
            assert_eq!(
                zone.sidecar(source.entity)
                    .unwrap()
                    .player
                    .as_ref()
                    .unwrap()
                    .pvp
                    .flag_until,
                Some(Tick(tick.tick.0 + 1200))
            );
            assert_eq!(zone.entities[&target.entity].combat.as_ref().unwrap().hp, hp);
            assert_eq!(zone.rng.get_word_pos() - cursor, if physical { 1 } else { 2 });
            assert!(zone
                .sidecar(target.entity)
                .unwrap()
                .effects
                .entries()
                .is_empty());
            let rows = tick
                .events
                .iter()
                .find_map(|e| {
                    if let ZoneEvent::SkillCastResult { targets, .. } = e {
                        Some(targets)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!(rows.iter().any(|r| r.target == target
                && r.outcome
                    == if physical {
                        SkillOutcome::Miss
                    } else {
                        SkillOutcome::Resist
                    }));
        }
    }
    #[test]
    fn review_spawn_contains_final_nonzero_status_before_effect_hydration() {
        use crate::interface::{grpc::pb, zone_mapping::observer_output_to_pb};
        let (mut zone, target, observer, _) = fixture();
        add_stun(&mut zone, target);
        let side = zone.sidecar_mut(target.entity).unwrap();
        let mut entries = side.effects.entries().clone();
        for effect in entries.values_mut() {
            effect.expires_at = Some(Tick(100));
        }
        side.effects = EffectList::from_entries(entries).unwrap();
        zone.known.clear();
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        let out = &tick.outputs[&observer.entity];
        let spawn = out.iter().position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn {entity,..}) if *entity == target.entity)).unwrap();
        let effect = out.iter().position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EffectApplied {target: t,..}) if *t == target)).unwrap();
        assert!(spawn < effect);
        let messages = observer_output_to_pb(&out[spawn], tick.server_time_ms);
        assert!(
            matches!(&messages[0].payload,Some(pb::server_message::Payload::Event(pb::WorldEvent { event:Some(pb::world_event::Event::Spawn(value)) })) if value.status_flags == effects::StatusFlags::STUN.bits())
        );
    }
    #[test]
    fn review_zero_damage_periodic_pulse_refreshes_hostility_without_rng() {
        let (mut zone, source, target, _) = fixture();
        review_learn(&mut zone, source.entity, 1168);
        let start = zone
            .run_tick(zone.draft(vec![review_input(
                source.entity,
                1,
                ZoneCommand::CastSkill {
                    entity: source.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1168),
                    target: Some(target.entity),
                },
            )]))
            .unwrap();
        assert!(start.dispositions.is_empty());
        let CastState::Casting(cast) = zone.sidecar(source.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        while zone.next_tick < cast.complete_at {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        loop {
            let mut preview = zone.clone();
            preview.roll_below(100);
            if preview.roll_below(100) < 10 {
                break;
            }
            zone.roll_below(100);
        }
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let due = zone
            .sidecar(target.entity)
            .unwrap()
            .effects
            .entries()
            .values()
            .flat_map(|e| &e.handlers)
            .find_map(|h| h.periodic.as_ref())
            .unwrap()
            .schedule
            .due_tick()
            .unwrap();
        while zone.next_tick < due {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        set_resources(&mut zone, target, 1, 10);
        let before = zone.rng.get_word_pos();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(zone.rng.get_word_pos(), before);
        assert_eq!(zone.entities[&target.entity].combat.as_ref().unwrap().hp, 1);
        assert_eq!(
            zone.sidecar(source.entity)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .pvp
                .flag_until,
            Some(Tick(due.0 + 1200))
        );
    }
    fn review_transfer_fixture() -> (ZoneState, TargetBinding, TargetBinding) {
        let (mut zone, source, observer, _) = fixture();
        let xp20 = zone.rules.as_ref().unwrap().xp_to_level(20).unwrap();
        if let CombatRole::Player { xp, .. } = &mut zone
            .entities
            .get_mut(&source.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        {
            *xp = xp20;
        }
        zone.relevel(source.entity).unwrap();
        zone.sync_passives_v5(source.entity).unwrap();
        zone.rebuild_v5(source.entity).unwrap();
        zone.reconcile_player_tokens(zone.next_tick, source.entity, None, &mut Vec::new());
        for (id, pos) in [
            (source.entity, Vec2Fixed::from_tiles(126, 128)),
            (observer.entity, Vec2Fixed::from_tiles(127, 128)),
        ] {
            let old = zone.entities[&id].pos;
            zone.entities.get_mut(&id).unwrap().pos = pos;
            zone.aoi.relocate(id, old, pos);
        }
        review_learn(&mut zone, source.entity, 1045);
        let started = zone
            .run_tick(zone.draft(vec![review_input(
                source.entity,
                1,
                ZoneCommand::CastSkill {
                    entity: source.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1045),
                    target: Some(source.entity),
                },
            )]))
            .unwrap();
        assert!(started.dispositions.is_empty(), "{:?}", started.dispositions);
        review_complete(&mut zone, source);
        assert!(!zone
            .sidecar(source.entity)
            .unwrap()
            .effects
            .entries()
            .is_empty());
        // The retained buff is external to the transferable paid skill ledger. The
        // helper temporarily grants its caster rank solely to exercise real onStart.
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut zone
            .entities
            .get_mut(&source.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            panic!("player")
        };
        p.class_state
            .learned_skills
            .retain(|s| s.key != "l2.skill.1045");
        p.class_state
            .learned_skills
            .iter_mut()
            .find(|s| s.key == "l2.skill.1177")
            .unwrap()
            .level = 2;
        let valid = p.class_state.validate_for_phase3(
            zone.classes.as_ref().unwrap(),
            &p.identity,
            crate::domain::CharacterId::from_uuid(source.entity.as_uuid()),
        );
        assert!(valid.is_ok(), "{valid:?}: {:?}", p.class_state);
        (zone, source, observer)
    }

    #[test]
    fn review_v5_class_transfer_preserves_paid_ranks_effects_ratios_and_frozen_retry() {
        use crate::domain::zone::transfer_resource;
        let (mut zone, source, observer) = review_transfer_fixture();
        let key = Uuid::from_u128(777);
        let command = ZoneCommand::ChangeClass {
            entity: source.entity,
            account: AccountId::from_uuid(source.entity.as_uuid()),
            request_key: key,
            target: ClassId(15),
        };
        let c = zone
            .entities
            .get_mut(&source.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        c.hp = c.sheet.max_hp() / 2;
        c.mp = c.sheet.max_mp() / 2;
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        else {
            panic!("player")
        };
        p.class_state.cp = p.max_cp / 2;
        let old = (c.hp, c.sheet.max_hp(), c.mp, c.sheet.max_mp(), p.class_state.cp, p.max_cp);
        let effects = zone.sidecar(source.entity).unwrap().effects.clone();
        let mut cold = ZoneState::from_snapshot(zone.snapshot()).unwrap();
        let inputs = vec![review_input(source.entity, 2, command.clone())];
        let tick = zone.run_tick(zone.draft(inputs.clone())).unwrap();
        assert_eq!(tick, cold.run_tick(cold.draft(inputs)).unwrap());
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let receipt = tick
            .events
            .iter()
            .find_map(|e| {
                if let ZoneEvent::ClassTransfer { receipt, .. } = e {
                    Some(receipt.as_ref().clone())
                } else {
                    None
                }
            })
            .unwrap();
        let c = zone.entities[&source.entity].combat.as_ref().unwrap();
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            panic!("player")
        };
        assert_eq!(p.class_state.current_class_id, ClassId(15));
        assert_eq!(p.class_state.token_tier_1_count, 0);
        assert!(p
            .class_state
            .learned_skills
            .iter()
            .any(|s| s.key == "l2.skill.1177" && s.level == 2));
        assert_eq!(zone.sidecar(source.entity).unwrap().effects, effects);
        assert_eq!(c.hp, transfer_resource(old.0, old.1, c.sheet.max_hp(), 1));
        assert_eq!(c.mp, transfer_resource(old.2, old.3, c.sheet.max_mp(), 0));
        assert_eq!(p.class_state.cp, transfer_resource(old.4, old.5, p.max_cp, 0));
        assert_eq!(
            (
                receipt.result.hp,
                receipt.result.max_hp,
                receipt.result.mp,
                receipt.result.max_mp,
                receipt.result.cp,
                receipt.result.max_cp
            ),
            (c.hp, c.sheet.max_hp(), c.mp, c.sheet.max_mp(), p.class_state.cp, p.max_cp)
        );
        assert!(!tick.outputs[&observer.entity].iter().any(|o|matches!(o,ObserverOutput::Event(ZoneEvent::StatsChanged {entity,..}) if *entity==source.entity)));
        assert!(tick.outputs[&observer.entity].iter().any(|o|matches!(o,ObserverOutput::Event(ZoneEvent::ClassChanged {entity,class_id,..}) if *entity==source.entity && *class_id==ClassId(15))));
        let mut cold = ZoneState::from_snapshot(zone.snapshot()).unwrap();
        for _ in 0..35 {
            assert_eq!(
                zone.run_tick(zone.draft(Vec::new())).unwrap(),
                cold.run_tick(cold.draft(Vec::new())).unwrap()
            );
        }
        let retry = zone
            .run_tick(zone.draft(vec![review_input(source.entity, 3, command)]))
            .unwrap();
        assert!(retry.dispositions.is_empty());
        assert!(!retry
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ClassTransfer { .. })));
        assert_eq!(
            zone.class_player(
                source.entity,
                AccountId::from_uuid(source.entity.as_uuid()),
                SessionGeneration(1)
            )
            .unwrap()
            .class_state
            .receipt(key),
            Some(&receipt)
        );
    }

    #[test]
    fn review_transfer_freezes_final_receipt_after_same_tick_buff_expiry() {
        let (mut zone, source, _) = review_transfer_fixture();
        let now = zone.next_tick;
        let side = zone.sidecar_mut(source.entity).unwrap();
        let mut entries = side.effects.entries().clone();
        for effect in entries.values_mut() {
            effect.expires_at = Some(now);
        }
        side.effects = EffectList::from_entries(entries).unwrap();
        let key = Uuid::from_u128(778);
        let tick = zone
            .run_tick(zone.draft(vec![review_input(
                source.entity,
                2,
                ZoneCommand::ChangeClass {
                    entity: source.entity,
                    account: AccountId::from_uuid(source.entity.as_uuid()),
                    request_key: key,
                    target: ClassId(15),
                },
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(zone
            .sidecar(source.entity)
            .unwrap()
            .effects
            .entries()
            .is_empty());
        let p = zone
            .class_player(
                source.entity,
                AccountId::from_uuid(source.entity.as_uuid()),
                SessionGeneration(1),
            )
            .unwrap();
        let result = &p.class_state.receipt(key).unwrap().result;
        let c = zone.entities[&source.entity].combat.as_ref().unwrap();
        assert_eq!((result.hp, result.max_hp), (c.hp, c.sheet.max_hp()));
        assert!(ZoneState::from_snapshot(zone.snapshot()).is_ok());
    }

    #[test]
    fn review_transfer_rejects_prior_staged_cast_and_invalid_target_without_partial_mutation() {
        for staged in [false, true] {
            let (mut zone, source, _) = review_transfer_fixture();
            let before = zone.snapshot();
            let mut commands = Vec::new();
            if staged {
                commands.push(review_input(
                    source.entity,
                    2,
                    ZoneCommand::CastSkill {
                        entity: source.entity,
                        skill_id: crate::domain::zone::skill::SkillId(1177),
                        target: Some(source.entity),
                    },
                ));
            }
            commands.push(review_input(
                source.entity,
                3,
                ZoneCommand::ChangeClass {
                    entity: source.entity,
                    account: AccountId::from_uuid(source.entity.as_uuid()),
                    request_key: Uuid::from_u128(779),
                    target: if staged { ClassId(15) } else { ClassId(1) },
                },
            ));
            let tick = zone.run_tick(zone.draft(commands)).unwrap();
            assert!(tick.dispositions.iter().any(|d| d.seq == Some(3)
                && d.reason
                    == if staged {
                        RejectReason::InCombat
                    } else {
                        RejectReason::TransferIneligible
                    }));
            assert!(!tick.events.iter().any(|e| matches!(
                e,
                ZoneEvent::ClassTransfer { .. } | ZoneEvent::EffectRemoved { .. }
            )));
            let original = before
                .entities
                .iter()
                .find(|e| e.id == source.entity)
                .unwrap();
            let c = zone.entities[&source.entity].combat.as_ref().unwrap();
            assert_eq!(c.role, original.combat.as_ref().unwrap().role);
        }
    }

    #[test]
    fn review_transfer_filters_nonretained_effects_without_replaying_start_hooks() {
        let (mut zone, source, _) = review_transfer_fixture();
        let side = zone.sidecar_mut(source.entity).unwrap();
        let mut entries = side.effects.entries().clone();
        // Explicit lifecycle-flag fixture: current executable buffs all retain on transfer.
        // Exercise the future data-driven false branch without changing registry input.
        for effect in entries.values_mut() {
            effect.flags.stay_on_subclass_change = false;
        }
        side.effects = EffectList::from_entries(entries).unwrap();
        let before = zone.rng.get_word_pos();
        let tick = zone
            .run_tick(zone.draft(vec![review_input(
                source.entity,
                2,
                ZoneCommand::ChangeClass {
                    entity: source.entity,
                    account: AccountId::from_uuid(source.entity.as_uuid()),
                    request_key: Uuid::from_u128(780),
                    target: ClassId(15),
                },
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(zone
            .sidecar(source.entity)
            .unwrap()
            .effects
            .entries()
            .is_empty());
        assert!(tick.events.iter().any(|e| matches!(
            e,
            ZoneEvent::EffectRemoved {
                reason: effects::EffectRemoveReason::Transfer,
                ..
            }
        )));
        assert!(!tick
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::EffectApplied { .. })));
        assert_eq!(zone.rng.get_word_pos(), before);
        assert!(ZoneState::from_snapshot(zone.snapshot()).is_ok());
    }
    fn item_intent(
        zone: &ZoneState,
        entity: EntityId,
        key: u128,
        kind: crate::domain::zone::ItemCommandKind,
    ) -> crate::domain::zone::ItemCommand {
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&entity].combat.as_ref().unwrap().role
        else {
            panic!("player")
        };
        crate::domain::zone::ItemCommand::new(entity, Uuid::from_u128(key), kind)
            .unwrap()
            .with_account(player.identity.account_id)
            .unwrap()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One recorded tick proves folding, scope/order, rejection rollback and cold replay.
    fn v6_equipment_commands_fold_in_order_and_freeze_one_final_tick_checkpoint() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, target, _) = equipment_fixture();
        // Finish the fixture's out-of-tick setup before taking a replay boundary.
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let original = zone.phase4.as_ref().unwrap().canonical.players[&target.entity].clone();
        let weapon = original.paperdoll[&EquipSlot::Rhand];
        let source_order = original.equipment_sources.clone();
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let before_rng = zone.rng.get_word_pos();
        let kinds = [
            (
                8,
                K::Unequip {
                    slot: EquipSlot::Lfinger,
                },
            ),
            (
                9,
                K::Unequip {
                    slot: EquipSlot::Rhand,
                },
            ),
            (
                9,
                K::Equip {
                    instance: weapon,
                    preferred_slot: None,
                },
            ),
            (
                9,
                K::Equip {
                    instance: weapon,
                    preferred_slot: None,
                },
            ),
            (
                9,
                K::Equip {
                    instance: weapon,
                    preferred_slot: Some(EquipSlot::Lhand),
                },
            ),
            (
                10,
                K::Equip {
                    instance: weapon,
                    preferred_slot: None,
                },
            ),
        ];
        let inputs = kinds
            .into_iter()
            .enumerate()
            .map(|(seq, (key, kind))| {
                ZoneInput::session(
                    target.entity,
                    SessionGeneration(1),
                    u32::try_from(seq.checked_add(1).unwrap()).unwrap(),
                    ZoneCommand::Item(Box::new(item_intent(&zone, target.entity, key, kind))),
                )
            })
            .collect();
        let draft = zone.draft(inputs);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied.dispositions.len(), 1);
        assert_eq!(applied.dispositions[0].reason, RejectReason::ItemOperationConflict);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(zone.rng.get_word_pos(), before_rng);
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&target.entity];
        assert_eq!(player.inventory_revision, original.inventory_revision.checked_add(1).unwrap());
        assert_eq!(player.paperdoll, original.paperdoll);
        assert_ne!(player.equipment_sources, source_order);
        let item_views: Vec<_> = applied
            .events
            .iter()
            .filter_map(|e| {
                if let ZoneEvent::ItemView(view) = e {
                    Some(&view.detail)
                } else {
                    None
                }
            })
            .collect();
        assert!(item_views.iter().any(|detail| matches!(detail,
            crate::domain::zone::item_views::ItemViewDetail::InventoryChanged { updated, .. }
            if updated.iter().any(|item| item.id == weapon))));
        assert!(!item_views.iter().any(|detail| matches!(
            detail,
            crate::domain::zone::item_views::ItemViewDetail::EquipmentChanged { .. }
        )));
        let checkpoints: Vec<_> = applied
            .events
            .iter()
            .filter_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(checkpoints.len(), 1);
        let checkpoint = checkpoints[0];
        assert_eq!(checkpoint.receipts.len(), 4);
        assert_eq!(checkpoint.ledger.len(), 2);
        assert_eq!(checkpoint.ledger[0].operation, ItemOperation::Unequip);
        assert_eq!(checkpoint.ledger[1].operation, ItemOperation::Equip);
        assert_eq!(checkpoint.receipts[1].reference().key, checkpoint.receipts[2].reference().key);
        assert_ne!(
            checkpoint.receipts[1].reference().operation,
            checkpoint.receipts[2].reference().operation
        );
        let results: Vec<_> = applied
            .events
            .iter()
            .filter_map(|event| {
                if let ZoneEvent::ItemResult(r) = event {
                    Some(r)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(results.len(), 5);
        assert_eq!(results[2], results[3]);
        assert_eq!(results[0].inventory_revision, original.inventory_revision);
        assert_eq!(results[1].inventory_revision, player.inventory_revision);
        let ordinary = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(target.entity, applied.tick)
            .unwrap();
        checkpoint
            .validate_structure(ordinary.modifier_install_order.len())
            .unwrap();
        for (observer, outputs) in &applied.outputs {
            if let Some(first) = outputs
                .iter()
                .position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::ItemResult(_))))
            {
                assert!(outputs[first..]
                    .iter()
                    .all(|o| matches!(o, ObserverOutput::Event(ZoneEvent::ItemResult(_)))));
            }
            assert!(outputs
                .iter()
                .all(|o| !matches!(o, ObserverOutput::Event(ZoneEvent::ItemCheckpoint(_)))));
            if *observer != target.entity {
                assert!(outputs
                    .iter()
                    .all(|o| !matches!(o, ObserverOutput::Event(ZoneEvent::ItemResult(_)))));
            }
        }
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
        let restored = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        assert_eq!(zone.snapshot(), restored.snapshot());
    }

    #[test]
    fn v6_equipment_command_rebuilds_before_next_command_and_rolls_back_each_refusal() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (mut zone, target, _) = equipment_fixture();
        let old_sheet = zone.effective(target.entity).unwrap();
        let old_hp = zone.entities[&target.entity].combat.as_ref().unwrap().hp;
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&target.entity].paperdoll
            [&EquipSlot::Rhand];
        let unequip = item_intent(
            &zone,
            target.entity,
            20,
            K::Unequip {
                slot: EquipSlot::Rhand,
            },
        );
        let mut events = Vec::new();
        let mut receipts = BTreeMap::new();
        zone.item_command_v6(zone.next_tick, &unequip, &mut events, &mut receipts)
            .unwrap();
        assert_ne!(
            zone.effective(target.entity).unwrap().physical.p_atk(),
            old_sheet.physical.p_atk()
        );
        assert!(zone.entities[&target.entity].combat.as_ref().unwrap().hp <= old_hp);
        let before = zone.snapshot();
        for kind in [
            K::Equip {
                instance: ItemInstanceId::new(99999).unwrap(),
                preferred_slot: None,
            },
            K::Equip {
                instance: weapon,
                preferred_slot: Some(EquipSlot::Lfinger),
            },
        ] {
            let command = item_intent(&zone, target.entity, 21, kind);
            let events_before = events.clone();
            assert!(zone
                .item_command_v6(zone.next_tick, &command, &mut events, &mut receipts)
                .is_err());
            assert_eq!(zone.snapshot(), before);
            assert_eq!(events, events_before);
        }
        let equip = item_intent(
            &zone,
            target.entity,
            21,
            K::Equip {
                instance: weapon,
                preferred_slot: None,
            },
        );
        zone.item_command_v6(zone.next_tick, &equip, &mut events, &mut receipts)
            .unwrap();
        assert_eq!(zone.effective(target.entity).unwrap().physical, old_sheet.physical);
        let sequence = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .next_install_sequence;
        let inventory = zone.phase4.as_ref().unwrap().canonical.players[&target.entity].clone();
        let noop = item_intent(
            &zone,
            target.entity,
            22,
            K::Equip {
                instance: weapon,
                preferred_slot: None,
            },
        );
        zone.item_command_v6(zone.next_tick, &noop, &mut events, &mut receipts)
            .unwrap();
        assert_eq!(
            zone.phase3
                .as_ref()
                .unwrap()
                .canonical
                .next_install_sequence,
            sequence
        );
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.players[&target.entity], inventory);
    }
    #[test]
    fn v6_equipment_race_and_active_item_cast_refusals_preserve_every_scalar() {
        use crate::domain::zone::{
            EquipSlot, ItemCommandKind as K, ItemInstance, ItemInstanceId, ItemLocation,
        };
        let (mut zone, owner, npc) = equipment_fixture();
        let restricted = ItemInstanceId::new(901).unwrap();
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .insert(
                restricted,
                ItemInstance {
                    creation_origin: None,
                    id: restricted,
                    template_id: 9210,
                    count: 1,
                    location: ItemLocation::Inventory,
                    slot: None,
                    enchant: 0,
                    augment_id: None,
                    charges: 0,
                    created_tick: Tick(0),
                    revision: 1,
                },
            );
        zone.rebuild_v5(owner.entity).unwrap();
        let command = item_intent(
            &zone,
            owner.entity,
            30,
            K::Equip {
                instance: restricted,
                preferred_slot: None,
            },
        );
        let before = zone.snapshot();
        let mut events = Vec::new();
        let mut receipts = BTreeMap::new();
        assert_eq!(
            zone.item_command_v6(zone.next_tick, &command, &mut events, &mut receipts),
            Err(RejectReason::ItemNotUsable)
        );
        assert_eq!(zone.snapshot(), before);
        assert!(events.is_empty());
        start_cast(&mut zone, owner, npc);
        let CastState::Casting(cast) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        zone.phase4.as_mut().unwrap().canonical.item_casts.insert(
            cast.id,
            crate::domain::zone::phase4::ItemCastState {
                caster: owner,
                cast: cast.clone(),
                origin: None,
                simultaneous: false,
                weapon: Some(weapon),
                shots: 0,
            },
        );
        let unequip = item_intent(
            &zone,
            owner.entity,
            31,
            K::Unequip {
                slot: EquipSlot::Rhand,
            },
        );
        let before = zone.snapshot();
        assert_eq!(
            zone.item_command_v6(zone.next_tick, &unequip, &mut events, &mut receipts),
            Err(RejectReason::AlreadyCasting)
        );
        assert_eq!(zone.snapshot(), before);
        assert!(events.is_empty());
        for kind in [
            K::Destroy {
                instance: weapon,
                count: 1,
            },
            K::Crystallize { instance: weapon },
        ] {
            let command = item_intent(&zone, owner.entity, 32, kind);
            assert_eq!(
                zone.item_command_v6(zone.next_tick, &command, &mut events, &mut receipts),
                Err(RejectReason::AlreadyCasting)
            );
            assert_eq!(zone.snapshot(), before);
            assert!(events.is_empty());
        }
        zone.phase4.as_mut().unwrap().canonical.item_casts.clear();
        // An ordinary cast with no weapon condition continues and reads the new sheet at impact.
        zone.item_command_v6(zone.next_tick, &unequip, &mut events, &mut receipts)
            .unwrap();
        assert_eq!(zone.sidecar(owner.entity).unwrap().cast, CastState::Casting(cast));
    }
    #[test]
    fn v6_enlarge_capacity_caller_uses_current_fold_and_keeps_legacy_limit() {
        use crate::domain::zone::effects::EffectCapacities;
        use crate::domain::zone::stat_functions::{
            FunctionKey, FunctionOp, StatCalculator, StatFunction,
        };
        let (mut zone, owner, _) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut stats = (*zone.effective(owner.entity).unwrap()).clone();
        stats.calculator = StatCalculator::try_new([StatFunction {
            key: FunctionKey {
                stat: "enlargeAbnormalSlot".into(),
                priority: 30,
                install_sequence: 1,
                handler_ordinal: 0,
                function_ordinal: 1,
            },
            operation: FunctionOp::Add(4 * Q),
            conditions: Vec::new(),
        }])
        .unwrap();
        zone.phase3
            .as_mut()
            .unwrap()
            .effective
            .insert(owner.entity, Arc::new(stats));
        assert_eq!(
            zone.effect_capacities_v6(owner.entity).unwrap(),
            EffectCapacities::for_enlarge(4)
        );
        zone.phase4 = None;
        assert_eq!(
            zone.effect_capacities_v6(owner.entity).unwrap(),
            EffectCapacities::for_enlarge(0)
        );
    }

    #[test]
    fn v6_weapon_context_tracks_equipment_for_conditional_getters() {
        use crate::domain::zone::skill::SkillCondition;
        use crate::domain::zone::stat_functions::{
            FunctionKey, FunctionOp, StatCalculator, StatFunction,
        };
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, npc) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        let mut stats = (*zone.effective(owner.entity).unwrap()).clone();
        stats.calculator = StatCalculator::try_new([StatFunction {
            key: FunctionKey {
                stat: "critDmg".into(),
                priority: 32,
                install_sequence: 1,
                handler_ordinal: 0,
                function_ordinal: 0,
            },
            operation: FunctionOp::Mul(2 * Q),
            conditions: vec![SkillCondition::UsingKind {
                weapon_kinds: vec!["FIST".into()],
            }],
        }])
        .unwrap();
        assert_eq!(zone.item_weapon_kind(owner.entity, "SWORD").unwrap(), "SWORD");
        assert_eq!(zone.item_weapon_kind(npc.entity, "SWORD").unwrap(), "NONE");
        assert_eq!(
            zone.damage_stat_v5(owner.entity, &stats, "critDmg", Q)
                .unwrap(),
            ExactRatio::whole(1)
        );
        for (key, kind, expected) in [
            (
                701,
                K::Unequip {
                    slot: EquipSlot::Rhand,
                },
                "FIST",
            ),
            (
                702,
                K::Equip {
                    instance: weapon,
                    preferred_slot: None,
                },
                "SWORD",
            ),
        ] {
            let command = item_intent(&zone, owner.entity, key, kind);
            let tick = zone
                .run_tick(zone.draft(vec![review_input(
                    owner.entity,
                    u64::try_from(key).unwrap(),
                    ZoneCommand::Item(Box::new(command)),
                )]))
                .unwrap();
            assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
            assert_eq!(zone.item_weapon_kind(owner.entity, "SWORD").unwrap(), expected);
            assert_eq!(
                zone.damage_stat_v5(owner.entity, &stats, "critDmg", Q)
                    .unwrap(),
                ExactRatio::whole(if expected == "FIST" { 2 } else { 1 })
            );
        }
        let (legacy, player, _, npc) = fixture();
        for entity in [player.entity, npc.entity] {
            for supplied in ["SWORD", "NONE"] {
                assert_eq!(legacy.item_weapon_kind(entity, supplied).unwrap(), supplied);
            }
        }
    }

    #[test]
    fn v6_allocator_extension_is_system_only_exhausted_atomic_and_replayed() {
        use crate::domain::zone::ItemInstanceRange;
        let (mut zone, owner, _) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let replacement = ItemInstanceRange::new(20000, 20000, 30000).unwrap();
        let before = zone.phase4.as_ref().unwrap().canonical.clone();
        let premature = zone
            .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::ExtendItemRange {
                range: replacement,
            })]))
            .unwrap();
        assert_eq!(premature.dispositions[0].reason, RejectReason::InvalidLoad);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical, before);
        zone.phase4.as_mut().unwrap().canonical.allocator =
            ItemInstanceRange::new(1, 10000, 10000).unwrap();
        let before = zone.phase4.as_ref().unwrap().canonical.clone();
        let rng = zone.rng.get_word_pos();
        for range in [
            ItemInstanceRange::new(9999, 9999, 11000).unwrap(),
            ItemInstanceRange::new(20000, 20001, 30000).unwrap(),
        ] {
            let rejected = zone
                .run_tick(
                    zone.draft(vec![ZoneInput::system(ZoneCommand::ExtendItemRange { range })]),
                )
                .unwrap();
            assert_eq!(rejected.dispositions[0].reason, RejectReason::InvalidLoad);
            assert_eq!(zone.phase4.as_ref().unwrap().canonical, before);
        }
        let rejected = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::ExtendItemRange { range: replacement },
            )]))
            .unwrap();
        assert_eq!(rejected.dispositions[0].reason, RejectReason::NotPermitted);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical, before);
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let draft = zone.draft(vec![ZoneInput::system(ZoneCommand::ExtendItemRange {
            range: replacement,
        })]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert!(applied.dispositions.is_empty());
        assert_eq!(zone.rng.get_word_pos(), rng);
        assert!(applied.events.is_empty());
        assert!(applied.outputs.values().all(Vec::is_empty));
        let mut expected = before;
        expected.allocator = replacement;
        assert_eq!(zone.phase4.as_ref().unwrap().canonical, expected);
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
        let restored = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        assert_eq!(restored.phase4.as_ref().unwrap().canonical.allocator, replacement);
        assert_eq!(restored.state_digest(), zone.state_digest());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // A session replacement and later equipment mutation exercise both owner boundaries.
    fn v6_inventory_replacement_baseline_and_deltas_preserve_privacy_and_replay() {
        use crate::domain::zone::item_views::{InventoryView, ItemViewDetail};
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, npc) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        start_cast(&mut zone, owner, npc);
        let expected = InventoryView::from_player(
            &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity],
            zone.phase4.as_ref().unwrap().rules.hash(),
        )
        .unwrap();
        let rng = zone.rng.get_word_pos();
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let draft = zone.draft(vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: owner.entity,
            generation: SessionGeneration(2),
        })]);
        let baseline = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(baseline, cold.run_tick(draft).unwrap());
        assert!(baseline.dispositions.is_empty());
        assert_eq!(zone.rng.get_word_pos(), rng);
        let outputs = &baseline.outputs[&owner.entity];
        let inventory_index = outputs
            .iter()
            .position(|output| {
                matches!(output,
            ObserverOutput::Event(ZoneEvent::ItemView(view))
            if matches!(&view.detail, ItemViewDetail::InventorySync { inventory, baseline_tick }
                if inventory == &expected && *baseline_tick == baseline.tick))
            })
            .unwrap();
        let skill_index = outputs
            .iter()
            .position(|output| {
                matches!(output, ObserverOutput::Event(ZoneEvent::SkillStateSync { .. }))
            })
            .unwrap();
        assert!(skill_index < inventory_index);
        for (index, output) in outputs.iter().enumerate() {
            if matches!(
                output,
                ObserverOutput::Event(
                    ZoneEvent::EntitySpawn { .. }
                        | ZoneEvent::EffectApplied { .. }
                        | ZoneEvent::SkillCastStarted { .. }
                        | ZoneEvent::AttackStarted { .. }
                )
            ) {
                assert!(index < skill_index);
            }
        }
        for (observer, outputs) in &baseline.outputs {
            if *observer != owner.entity {
                assert!(!outputs.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::ItemView(view)) if view.entity == owner.entity && view.owner_only())));
            }
        }
        let command = item_intent(
            &zone,
            owner.entity,
            123,
            K::Unequip {
                slot: EquipSlot::Rhand,
            },
        );
        let draft = zone.draft(vec![ZoneInput::session(
            owner.entity,
            SessionGeneration(2),
            1,
            ZoneCommand::Item(Box::new(command)),
        )]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert!(applied.dispositions.is_empty());
        let mut observers = 0;
        for (observer, outputs) in &applied.outputs {
            let views: Vec<_> = outputs
                .iter()
                .filter_map(|o| {
                    if let ObserverOutput::Event(ZoneEvent::ItemView(view)) = o {
                        Some(view)
                    } else {
                        None
                    }
                })
                .collect();
            assert!(views.iter().any(|view| matches!(&view.detail, ItemViewDetail::EquipmentChanged { visible }
                if visible.iter().any(|entry| entry.slot == EquipSlot::Rhand && entry.template_id == 0 && entry.glow_tier == 0))));
            if *observer == owner.entity {
                assert!(views.iter().any(|view| matches!(&view.detail, ItemViewDetail::InventoryChanged { previous_revision, inventory_revision, .. }
                    if *previous_revision == expected.inventory_revision && Some(*inventory_revision) == previous_revision.checked_add(1))));
                let result = outputs
                    .iter()
                    .position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::ItemResult(_))))
                    .unwrap();
                assert!(outputs
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| matches!(o, ObserverOutput::Event(ZoneEvent::ItemView(_))))
                    .all(|(index, _)| index < result));
            } else {
                observers += 1;
                assert!(views.iter().all(|view| !view.owner_only()));
            }
        }
        assert!(observers > 0);
        for applied in [baseline, applied] {
            let record = crate::application::replay_log::AppliedTickRecord::from_applied(
                zone.seed.zone,
                &applied,
            );
            assert_eq!(
                crate::application::replay_log::AppliedTickRecord::decode(&record.encode())
                    .unwrap(),
                record
            );
        }
    }
    #[test]
    #[allow(clippy::too_many_lines)] // Durable receipt retry after death must retain every original result field.
    fn v6_recorded_known_item_result_survives_dead_eligibility_without_reexecution() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, npc) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let command = item_intent(
            &zone,
            owner.entity,
            99,
            K::Unequip {
                slot: EquipSlot::Rhand,
            },
        );
        let tick = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::Item(Box::new(command.clone())),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty());
        let original = tick
            .events
            .iter()
            .find_map(|e| {
                if let ZoneEvent::ItemResult(result) = e {
                    Some(result.as_ref().clone())
                } else {
                    None
                }
            })
            .unwrap();
        let payload = original.payload().unwrap();
        let known = command
            .clone()
            .with_known_receipt(
                item_intent(
                    &zone,
                    owner.entity,
                    99,
                    K::Unequip {
                        slot: EquipSlot::Rhand,
                    },
                )
                .fingerprint_hex()
                .unwrap(),
                payload.clone(),
            )
            .unwrap();
        set_resources(&mut zone, owner, 1, 0);
        zone.apply_damage_rolled_v5(
            Tick(3),
            DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target: owner,
                cause: DamageCause::Direct,
                rolled_damage: 50,
            },
            &mut Vec::new(),
            &mut scripted(&[(DamageRollSite::DeathPenalty, 100, 99)], &mut Vec::new()),
        )
        .unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(zone.entities[&owner.entity].combat.as_ref().unwrap().hp, 0);
        let before = zone.phase4.as_ref().unwrap().canonical.clone();
        let rng = zone.rng.get_word_pos();
        let sequence = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .next_install_sequence;
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let mut conflicting = command.clone();
        conflicting.kind = K::Unequip {
            slot: EquipSlot::Lhand,
        };
        let recorded_conflict = conflicting
            .clone()
            .with_receipt_conflict(command.fingerprint_hex().unwrap())
            .unwrap();
        let draft = zone.draft(vec![
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                10,
                ZoneCommand::Item(Box::new(recorded_conflict)),
            ),
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                11,
                ZoneCommand::Item(Box::new(conflicting.clone())),
            ),
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                2,
                ZoneCommand::Item(Box::new(known.clone())),
            ),
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                3,
                ZoneCommand::Item(Box::new(command.clone())),
            ),
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                4,
                ZoneCommand::Item(Box::new(conflicting)),
            ),
        ]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(applied.dispositions.len(), 3);
        assert!(applied
            .dispositions
            .iter()
            .all(|d| d.reason == RejectReason::ItemOperationConflict));
        assert_eq!(zone.phase4.as_ref().unwrap().canonical, before);
        assert_eq!(zone.rng.get_word_pos(), rng);
        assert_eq!(
            zone.phase3
                .as_ref()
                .unwrap()
                .canonical
                .next_install_sequence,
            sequence
        );
        assert!(!applied
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(_))));
        let returned: Vec<_> = applied
            .events
            .iter()
            .filter_map(|e| {
                if let ZoneEvent::ItemResult(result) = e {
                    Some(result.as_ref())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(returned.len(), 2);
        for result in returned {
            assert_eq!(result, &original);
            assert_eq!(result.payload().unwrap().as_bytes(), payload.as_bytes());
            assert!(result.tick < applied.tick);
        }
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
        let fresh = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                5,
                ZoneCommand::Item(Box::new(command)),
            )]))
            .unwrap();
        assert_eq!(fresh.dispositions[0].reason, RejectReason::DeadActor);
        assert!(!fresh
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemResult(_))));
        let rejected = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(0),
                3,
                ZoneCommand::Item(Box::new(known)),
            )]))
            .unwrap();
        assert_eq!(rejected.dispositions[0].reason, RejectReason::StaleSession);
        assert!(!rejected
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemResult(_) | ZoneEvent::ItemCheckpoint(_))));
    }

    fn crystallize_fixture() -> (ZoneState, TargetBinding) {
        let (mut zone, owner, _) = equipment_fixture();
        let xp40 = zone.rules.as_ref().unwrap().xp_to_level(40).unwrap();
        let entity = zone.entities.get_mut(&owner.entity).unwrap();
        let combat = entity.combat.as_mut().unwrap();
        let CombatRole::Player {
            progression: Some(progression),
            class,
            xp,
        } = &mut combat.role
        else {
            unreachable!()
        };
        progression.identity.race = Race::Dwarf;
        progression.identity.base_class_id = ClassId(53);
        progression.class_state = ClassState::new(ClassId(53));
        *class = crate::domain::character_progression::base_class_profile(ClassId(53))
            .unwrap()
            .into();
        *xp = xp40;
        let (sheet, max_cp, _, radius) = crate::domain::zone::profession_stats(
            zone.rules.as_ref().unwrap(),
            zone.classes.as_ref().unwrap(),
            &progression.identity,
            &progression.class_state,
            40,
        )
        .unwrap();
        progression.max_cp = max_cp;
        combat.sheet = sheet;
        combat.collision_radius = radius;
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .starter_kit_receipt
            .source_class = ClassId(53);
        zone.sync_passives_v5(owner.entity).unwrap();
        zone.rebuild_v5(owner.entity).unwrap();
        zone.reconcile_player_tokens(zone.next_tick, owner.entity, None, &mut Vec::new());
        let old = zone.entities[&owner.entity].pos;
        let pos = Vec2Fixed::from_tiles(126, 128);
        zone.entities.get_mut(&owner.entity).unwrap().pos = pos;
        zone.aoi.relocate(owner.entity, old, pos);
        for (seq, target) in [(100, ClassId(54)), (101, ClassId(55))] {
            let command = ZoneCommand::ChangeClass {
                entity: owner.entity,
                account: AccountId::from_uuid(owner.entity.as_uuid()),
                request_key: Uuid::from_u128(u128::from(seq)),
                target,
            };
            let applied = zone
                .run_tick(zone.draft(vec![review_input(owner.entity, seq, command)]))
                .unwrap();
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        }
        (zone, owner)
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Real command ordering, provider removal and cold replay share one source fixture.
    fn v6_removal_commands_unequip_refund_and_replay_without_rng() {
        use crate::domain::zone::phase4_projection::ItemOperation as Op;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (mut zone, owner) = crystallize_fixture();
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            unreachable!()
        };
        progression
            .class_state
            .merge_learned_skills([crate::domain::subclass::LearnedSkill {
                key: "l2.skill.248".into(),
                level: 2,
            }]);
        let items = &mut zone.phase4.as_mut().unwrap().canonical;
        let player = items.players.get_mut(&owner.entity).unwrap();
        let weapon = player.paperdoll[&EquipSlot::Rhand];
        let chest = player.paperdoll[&EquipSlot::Chest];
        let arrows = ItemInstanceId::new(900).unwrap();
        let arrow_count = player.inventory[&arrows].count;
        let crystal = ItemInstanceId::new(901).unwrap();
        let mut stack = player.inventory[&arrows].clone();
        stack.id = crystal;
        stack.template_id = 1459;
        stack.count = 10;
        player.inventory.insert(crystal, stack);
        zone.sync_passives_v5(owner.entity).unwrap();
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let original = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
        let allocation = zone.phase4.as_ref().unwrap().canonical.allocator;
        let rng = zone.rng.get_word_pos();
        zone.validate_runtime_v5().unwrap();
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let commands = [
            (
                20,
                K::Destroy {
                    instance: arrows,
                    count: 3,
                },
            ),
            (
                20,
                K::Destroy {
                    instance: arrows,
                    count: 3,
                },
            ),
            (
                21,
                K::Destroy {
                    instance: chest,
                    count: 1,
                },
            ),
            (22, K::Crystallize { instance: weapon }),
        ]
        .into_iter()
        .enumerate()
        .map(|(seq, (key, kind))| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                u32::try_from(seq + 1).unwrap(),
                ZoneCommand::Item(Box::new(item_intent(&zone, owner.entity, key, kind))),
            )
        })
        .collect();
        let draft = zone.draft(commands);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(zone.rng.get_word_pos(), rng);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.allocator, allocation);
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory_revision, original.inventory_revision + 1);
        assert_eq!(player.inventory[&arrows].count, arrow_count - 3);
        // Flamberge 705 + C weapon bonus45*(2*4-3) = 930; merge into original10.
        assert_eq!(player.inventory[&crystal].count, 940);
        assert!(!player.inventory.contains_key(&weapon));
        assert!(!player.inventory.contains_key(&chest));
        assert!(!player.paperdoll.contains_key(&EquipSlot::Rhand));
        assert!(!player.paperdoll.contains_key(&EquipSlot::Chest));
        assert!(player.active_sets.is_empty());
        assert!(!player.equipment_sources.values().any(|source| matches!(source, crate::domain::zone::phase4_checkpoint::EquipmentSourceKey::Item { instance } if *instance == weapon || *instance == chest)));
        let projection = applied
            .events
            .iter()
            .find_map(|e| {
                if let ZoneEvent::ItemCheckpoint(p) = e {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.receipts.len(), 3);
        assert_eq!(
            projection
                .ledger
                .iter()
                .map(|delta| delta.operation)
                .collect::<Vec<_>>(),
            vec![
                Op::Destroy,
                Op::Unequip,
                Op::Destroy,
                Op::Unequip,
                Op::Destroy,
                Op::Grant
            ]
        );
        assert_eq!(projection.ledger.last().unwrap().receipt.unwrap().operation, Op::Crystallize);
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
        let restored = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        assert_eq!(zone.snapshot(), restored.snapshot());
    }

    #[test]
    fn v6_removal_refusals_preserve_items_providers_allocator_and_receipts() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (mut zone, owner, _) = equipment_fixture();
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        let arrows = ItemInstanceId::new(900).unwrap();
        let count =
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&arrows].count;
        let cases = [
            (K::Crystallize { instance: weapon }, RejectReason::CrystallizeLevelTooLow),
            (K::Crystallize { instance: arrows }, RejectReason::ItemNotUsable),
            (
                K::Destroy {
                    instance: arrows,
                    count: count + 1,
                },
                RejectReason::InsufficientItems,
            ),
            (
                K::Destroy {
                    instance: weapon,
                    count: 2,
                },
                RejectReason::InsufficientItems,
            ),
            (
                K::Destroy {
                    instance: ItemInstanceId::new(999).unwrap(),
                    count: 1,
                },
                RejectReason::ItemNotOwned,
            ),
        ];
        let mut events = Vec::new();
        let mut receipts = BTreeMap::new();
        for (kind, expected) in cases {
            let snapshot = zone.snapshot();
            let command = item_intent(&zone, owner.entity, 20, kind);
            assert_eq!(
                zone.item_command_v6(zone.next_tick, &command, &mut events, &mut receipts),
                Err(expected)
            );
            assert_eq!(zone.snapshot(), snapshot);
            assert!(events.is_empty());
            assert!(receipts.is_empty());
        }
        let CombatRole::Player {
            progression: Some(progression),
            ..
        } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            unreachable!()
        };
        progression
            .class_state
            .merge_learned_skills([crate::domain::subclass::LearnedSkill {
                key: "l2.skill.248".into(),
                level: 2,
            }]);
        // The refund cannot occupy the retained zero-slot inventory. No unequip survives.
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .ordinary_capacity = 0;
        let snapshot = zone.snapshot();
        let command = item_intent(&zone, owner.entity, 20, K::Crystallize { instance: weapon });
        assert_eq!(
            zone.item_command_v6(zone.next_tick, &command, &mut events, &mut receipts),
            Err(RejectReason::InventoryFull)
        );
        assert_eq!(zone.snapshot(), snapshot);
        // Removal remains possible with a nonpositive retained weight/capacity ceiling.
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .max_load = -1;
        let command = item_intent(
            &zone,
            owner.entity,
            20,
            K::Destroy {
                instance: arrows,
                count,
            },
        );
        zone.item_command_v6(zone.next_tick, &command, &mut events, &mut receipts)
            .unwrap();
        assert!(!zone.phase4.as_ref().unwrap().canonical.players[&owner.entity]
            .inventory
            .contains_key(&arrows));
    }
    fn enchant_fixture(
        scroll_template: u32,
        slot: crate::domain::zone::EquipSlot,
        enchant: u32,
    ) -> (
        ZoneState,
        TargetBinding,
        crate::domain::zone::ItemInstanceId,
        crate::domain::zone::ItemInstanceId,
    ) {
        use crate::domain::zone::{item_sources, ItemInstanceId};
        let (mut zone, owner, _) = equipment_fixture();
        let items = zone.phase4.as_mut().unwrap();
        let player = items.canonical.players.get_mut(&owner.entity).unwrap();
        let target = player.paperdoll[&slot];
        player.inventory.get_mut(&target).unwrap().enchant = enchant;
        let scroll = ItemInstanceId::new(901).unwrap();
        let mut stack = player.inventory[&ItemInstanceId::new(900).unwrap()].clone();
        stack.id = scroll;
        stack.template_id = scroll_template;
        stack.count = 3;
        player.inventory.insert(scroll, stack);
        player.active_sets = item_sources::reconcile_sets(player, &items.rules, None).unwrap();
        item_sources::reconcile_sources(
            player,
            &items.rules,
            &mut zone
                .phase3
                .as_mut()
                .unwrap()
                .canonical
                .next_install_sequence,
        )
        .unwrap();
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        zone.validate_runtime_v5().unwrap();
        (zone, owner, target, scroll)
    }

    #[test]
    #[allow(clippy::too_many_lines)] // All source outcomes use the real command, private audit, and cold replay.
    fn v6_enchant_outcomes_pay_once_draw_once_and_cold_replay() {
        use crate::domain::zone::phase4_projection::{
            ItemEnchantOutcome as O, ItemOperation, ItemResultDetail,
        };
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        for (template, slot, enchant, chance, success, expected, refund) in [
            (951, EquipSlot::Rhand, 4, 70_000, true, O::Success, 0),
            (951, EquipSlot::Rhand, 4, 70_000, false, O::Destroyed, 577),
            (22227, EquipSlot::Rhand, 4, 70_000, false, O::SafeFailed, 0),
            (6573, EquipSlot::Rhand, 4, 70_000, false, O::BlessedReset, 0),
            (22010, EquipSlot::Chest, 20, 10_000, true, O::SuccessNoIncrement, 0),
            (956, EquipSlot::Chest, 0, 100_000, true, O::Success, 0),
            (956, EquipSlot::Chest, 20, 0, false, O::Destroyed, 724),
        ] {
            let (mut zone, owner, target, scroll) = enchant_fixture(template, slot, enchant);
            // Select a deterministic fixture stream before admission; production never retries a draw.
            let raw = loop {
                let raw = zone.rng.clone().next_u32();
                if (raw % 100_000 < chance) == success {
                    break raw;
                }
                zone.rng.next_u32();
            };
            let cursor = zone.rng.get_word_pos();
            let original = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
            let mut cold = ZoneState::from_snapshot_with_items(
                zone.snapshot(),
                Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
            )
            .unwrap();
            let intent = item_intent(
                &zone,
                owner.entity,
                30,
                K::Enchant {
                    target,
                    scroll,
                    support: None,
                },
            );
            let draft = zone.draft(
                (1..=2)
                    .map(|seq| {
                        ZoneInput::session(
                            owner.entity,
                            SessionGeneration(1),
                            seq,
                            ZoneCommand::Item(Box::new(intent.clone())),
                        )
                    })
                    .collect(),
            );
            let applied = zone.run_tick(draft.clone()).unwrap();
            assert!(
                applied.dispositions.is_empty(),
                "scroll {template}: {:?}",
                applied.dispositions
            );
            assert_eq!(applied, cold.run_tick(draft).unwrap());
            assert_eq!(zone.rng.get_word_pos(), cursor + 1);
            let projection = applied
                .events
                .iter()
                .find_map(|event| {
                    if let ZoneEvent::ItemCheckpoint(p) = event {
                        Some(p)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(projection.receipts.len(), 1);
            assert_eq!(projection.enchant_attempts.len(), 1);
            let attempt = &projection.enchant_attempts[0];
            assert_eq!(
                (attempt.raw_draw, attempt.draw, attempt.chance_1e5, attempt.outcome),
                (raw, raw % 100_000, chance, expected)
            );
            assert_eq!(attempt.before, original.inventory[&target]);
            let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
            assert_eq!(player.inventory[&scroll].count, 2);
            assert_eq!(player.inventory_revision, original.inventory_revision + 1);
            assert_eq!(attempt.after.as_ref(), player.inventory.get(&target));
            if matches!(expected, O::SafeFailed | O::SuccessNoIncrement) {
                assert_eq!(player.inventory[&target], original.inventory[&target]);
            }
            if matches!(expected, O::BlessedReset | O::Destroyed) {
                assert!(!player.paperdoll.contains_key(&slot));
            }
            if expected == O::Success {
                assert_eq!(player.inventory[&target].enchant, enchant + 1);
            }
            if expected == O::BlessedReset {
                assert_eq!(player.inventory[&target].enchant, 0);
            }
            let ItemResultDetail::Enchant { refunds, .. } = &projection.receipts[0].result.detail
            else {
                unreachable!()
            };
            assert_eq!(refunds.iter().map(|r| r.count).sum::<u64>(), refund);
            let consume = projection
                .ledger
                .iter()
                .filter(|row| row.operation == ItemOperation::Consume)
                .collect::<Vec<_>>();
            assert_eq!(consume.len(), 1);
            assert_eq!(
                (consume[0].instance, consume[0].count_delta, consume[0].causes.len()),
                (scroll, -1, 1)
            );
            assert_eq!(
                applied
                    .events
                    .iter()
                    .filter(|event| matches!(event, ZoneEvent::ItemResult(_)))
                    .count(),
                2
            );
            let record = crate::application::replay_log::AppliedTickRecord::from_applied(
                zone.seed.zone,
                &applied,
            );
            assert_eq!(
                crate::application::replay_log::AppliedTickRecord::decode(&record.encode())
                    .unwrap(),
                record
            );
            let restored = ZoneState::from_snapshot_with_items(
                zone.snapshot(),
                Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
            )
            .unwrap();
            assert_eq!(zone.snapshot(), restored.snapshot());
        }
    }

    #[test]
    fn v6_enchant_captures_each_attempt_before_later_destruction_and_aggregates_payments() {
        use crate::domain::zone::phase4_projection::{
            ItemEnchantOutcome as O, ItemOperation as Op,
        };
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, target, scroll) = enchant_fixture(956, EquipSlot::Chest, 0);
        let cursor = zone.rng.get_word_pos();
        let draft = zone.draft(
            [
                K::Enchant {
                    target,
                    scroll,
                    support: None,
                },
                K::Enchant {
                    target,
                    scroll,
                    support: None,
                },
                K::Destroy {
                    instance: target,
                    count: 1,
                },
            ]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                ZoneInput::session(
                    owner.entity,
                    SessionGeneration(1),
                    u32::try_from(index + 1).unwrap(),
                    ZoneCommand::Item(Box::new(item_intent(
                        &zone,
                        owner.entity,
                        u128::try_from(index + 1).unwrap(),
                        kind,
                    ))),
                )
            })
            .collect(),
        );
        let applied = zone.run_tick(draft).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(zone.rng.get_word_pos(), cursor + 2);
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.receipts.len(), 3);
        assert_eq!(projection.enchant_attempts.len(), 2);
        assert!(!projection
            .checkpoint
            .instances
            .iter()
            .any(|item| item.id == target));
        for (index, attempt) in projection.enchant_attempts.iter().enumerate() {
            assert_eq!(attempt.outcome, O::Success);
            assert_eq!(attempt.old_enchant, u32::try_from(index).unwrap());
            assert_eq!(attempt.after.as_ref().unwrap().enchant, u32::try_from(index + 1).unwrap());
        }
        let consume = projection
            .ledger
            .iter()
            .filter(|row| row.operation == Op::Consume)
            .collect::<Vec<_>>();
        assert_eq!(consume.len(), 1);
        assert_eq!((consume[0].count_delta, consume[0].causes.len()), (-2, 2));
        assert_eq!(consume[0].before.as_ref().unwrap().count, 3);
        assert_eq!(consume[0].after.as_ref().unwrap().count, 1);
    }

    #[test]
    fn v6_enchant_refuses_invalid_inputs_and_unpayable_refunds_without_draws() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        for scenario in 0..4 {
            let (mut zone, owner, target, scroll) = enchant_fixture(
                if scenario == 0 {
                    955
                } else if scenario == 1 {
                    22227
                } else {
                    951
                },
                EquipSlot::Rhand,
                if scenario == 1 { 15 } else { 4 },
            );
            if scenario == 3 {
                // Scroll remains a stack and destroying a held target frees one slot; weight alone blocks refund.
                let player = zone
                    .phase4
                    .as_mut()
                    .unwrap()
                    .canonical
                    .players
                    .get_mut(&owner.entity)
                    .unwrap();
                player.max_load = -1;
            }
            let original = zone.phase4.as_ref().unwrap().canonical.clone();
            let phase3 = zone.phase3.as_ref().unwrap().canonical.clone();
            let cursor = zone.rng.get_word_pos();
            let intent = item_intent(
                &zone,
                owner.entity,
                40,
                K::Enchant {
                    target,
                    scroll,
                    support: (scenario == 2).then(|| ItemInstanceId::new(999).unwrap()),
                },
            );
            let mut events = Vec::new();
            let result = zone.item_command_v6(Tick(20), &intent, &mut events, &mut BTreeMap::new());
            assert!(result.is_err(), "scenario {scenario}");
            assert!(events.is_empty());
            assert_eq!(zone.rng.get_word_pos(), cursor);
            assert_eq!(zone.phase4.as_ref().unwrap().canonical, original);
            assert_eq!(zone.phase3.as_ref().unwrap().canonical, phase3);
        }
    }
    fn item_cast_fixture() -> (ZoneState, TargetBinding, crate::domain::zone::CastId) {
        use crate::domain::zone::phase4::{ItemCastOrigin, ItemCastState};
        use crate::domain::zone::{skill::SkillId, ItemInstanceId};
        let (mut zone, owner, _) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let now = Tick(zone.next_tick.0.checked_sub(1).unwrap());
        let rules = Arc::clone(&zone.phase3.as_ref().unwrap().canonical.rules);
        let mut next = zone.phase3.as_ref().unwrap().canonical.next_cast_id;
        let mut main = None;
        for (template, skill, rank, simultaneous) in [(737, 2014, 1, false), (5591, 2166, 1, true)]
        {
            let skill = rules.skills().get(SkillId(skill)).unwrap();
            let mut local = zone.sidecar(owner.entity).unwrap().clone();
            local.cast = CastState::Idle;
            let effective = zone.effective(owner.entity).unwrap();
            let terms = state_skill::cast_terms(
                skill,
                rank,
                CastStatContext {
                    world: None,
                    player_level: Some(effective.physical.level()),
                    calculator: &effective.calculator,
                    conditions: FunctionConditions {
                        weapon_kind: "SWORD",
                        can_resurrect: true,
                    },
                    attack_speed: 1,
                    cast_speed: effective.cast_speed,
                    flags: effective.flags,
                },
            )
            .unwrap();
            let mut mp = zone.entities[&owner.entity].combat.as_ref().unwrap().mp;
            let cast = local
                .begin_cast(
                    &mut next,
                    &mut mp,
                    zone.entities[&owner.entity].combat.as_ref().unwrap().hp,
                    0,
                    CastStart {
                        now,
                        ordinal: Ordinal(1),
                        incarnation: owner.incarnation,
                        terms,
                        target: Some(owner),
                        affect_limit: 0,
                        targets: vec![owner],
                    },
                )
                .unwrap();
            local.launch_cast(now, cast.id, |_| true);
            let CastState::Casting(cast) = local.cast else {
                unreachable!()
            };
            if !simultaneous {
                main = Some(cast.clone());
            }
            zone.phase4.as_mut().unwrap().canonical.item_casts.insert(
                cast.id,
                ItemCastState {
                    caster: owner,
                    cast,
                    origin: Some(ItemCastOrigin {
                        instance: ItemInstanceId::new(if simultaneous { 902 } else { 901 })
                            .unwrap(),
                        template,
                        count: 1,
                        skill_ordinal: 0,
                        receipt: crate::domain::zone::phase4_projection::ItemReceiptRef {
                            operation: crate::domain::zone::phase4_projection::ItemOperation::Use,
                            key: Uuid::from_u128(u128::from(template)),
                        },
                    }),
                    simultaneous,
                    weapon: None,
                    shots: 0,
                },
            );
        }
        let main = main.unwrap();
        let main_id = main.id;
        let phase = &mut zone.phase3.as_mut().unwrap().canonical;
        phase.next_cast_id = next;
        phase.entities.get_mut(&owner.entity).unwrap().cast = CastState::Casting(main);
        (zone, owner, main_id)
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Synthetic retained-state boundary test; actual Use producer is a separate gate.
    fn v6_item_cast_snapshot_requires_exact_paid_source_and_preserves_both_lanes() {
        use crate::domain::zone::skill::{SkillId, SkillRef};
        let (zone, owner, main_id) = item_cast_fixture();
        let now = Tick(zone.next_tick.0.checked_sub(1).unwrap());
        let cursor = zone.rng.get_word_pos();
        let CastState::Casting(main) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
            unreachable!()
        };
        // The item skill must never be inserted into the permanent class ledger.
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&owner.entity].combat.as_ref().unwrap().role
        else {
            unreachable!()
        };
        assert!(!player
            .class_state
            .learned_skills
            .iter()
            .any(|entry| entry.key == "l2.skill.2014"));
        zone.validate_runtime_v5().unwrap();
        let snapshot = zone.snapshot();
        let item_rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let restored =
            ZoneState::from_snapshot_with_items(snapshot.clone(), Some(Arc::clone(&item_rules)))
                .unwrap();
        assert_eq!(snapshot, restored.snapshot());
        assert_eq!(zone.rng.get_word_pos(), cursor);
        assert_eq!(zone.sidecar(owner.entity).unwrap().cast, CastState::Casting(main));
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.item_casts.len(), 2);
        for mutation in 0..11 {
            let mut invalid = snapshot.clone();
            let items = invalid.phase4.as_mut().unwrap();
            if mutation == 0 {
                items.item_casts.remove(&main_id);
            } else {
                let lane = items.item_casts.get_mut(&main_id).unwrap();
                match mutation {
                    1 => lane.origin = None,
                    2 => lane.origin.as_mut().unwrap().template = 951,
                    3 => lane.origin.as_mut().unwrap().count = 0,
                    4 => lane.caster.incarnation += 1,
                    5 => lane.simultaneous = true,
                    6 => lane.shots = 8,
                    7 => {
                        lane.cast.skill = SkillRef {
                            id: SkillId(2014),
                            rank: 2,
                        }
                    },
                    8 => lane.origin.as_mut().unwrap().skill_ordinal = 1,
                    9 => {
                        lane.origin.as_mut().unwrap().receipt.operation =
                            crate::domain::zone::phase4_projection::ItemOperation::Enchant;
                    },
                    10 => lane.origin.as_mut().unwrap().receipt.key = Uuid::nil(),
                    _ => unreachable!(),
                }
            }
            assert!(
                ZoneState::from_snapshot_with_items(invalid, Some(Arc::clone(&item_rules)))
                    .is_err(),
                "mutation {mutation}"
            );
        }
        let mut invalid = snapshot;
        let simultaneous = invalid
            .phase4
            .as_mut()
            .unwrap()
            .item_casts
            .values_mut()
            .find(|lane| lane.simultaneous)
            .unwrap();
        simultaneous.cast.complete_at = now;
        assert!(ZoneState::from_snapshot_with_items(invalid, Some(item_rules)).is_err());
    }
    #[test]
    fn v6_item_cast_auxiliary_completes_once_without_replacing_main_and_cold_replays() {
        let (mut zone, owner, main_id) = item_cast_fixture();
        let main = zone.sidecar(owner.entity).unwrap().cast.clone();
        let auxiliary = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .values()
            .find(|lane| lane.simultaneous)
            .unwrap()
            .cast
            .id;
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            unreachable!()
        };
        player.class_state.cp = 0;
        zone.sidecar_mut(owner.entity).unwrap().fractions.cp = 0;
        let expected = zone.effective(owner.entity).unwrap().max_cp.min(50);
        let cursor = zone.rng.get_word_pos();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = zone.draft(Vec::new());
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        let results: Vec<_> = applied
            .events
            .iter()
            .filter_map(|event| match event {
                ZoneEvent::SkillCastResult {
                    cast_id, targets, ..
                } if *cast_id == auxiliary => Some(targets),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 1);
        // Public heal counts expose HP only; CP restoration stays in owner resources.
        assert_eq!(results[0][0].heal, 0);
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&owner.entity].combat.as_ref().unwrap().role
        else {
            unreachable!()
        };
        assert_eq!(player.class_state.cp, expected);
        assert_eq!(zone.sidecar(owner.entity).unwrap().cast, main);
        assert_eq!(zone.rng.get_word_pos(), cursor);
        assert_eq!(
            zone.phase4
                .as_ref()
                .unwrap()
                .canonical
                .item_casts
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![main_id]
        );
        zone.validate_runtime_v5().unwrap();
        let restored = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        assert_eq!(zone.snapshot(), restored.snapshot());
        let next = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert!(!next.events.iter().any(|event| matches!(event, ZoneEvent::SkillCastResult { cast_id, .. } if *cast_id == auxiliary)));
    }

    #[test]
    fn v6_item_cast_stop_cancels_both_lanes_and_lost_main_target_removes_its_context() {
        let (mut zone, owner, main_id) = item_cast_fixture();
        let mut stopped = zone.clone();
        let tick = stopped
            .run_tick(stopped.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::StopMove {
                    entity: owner.entity,
                },
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty());
        let cancelled = tick.events.iter().filter(|event| matches!(event, ZoneEvent::SkillCastCancelled { caster, reason: CastCancelReason::Moved, .. } if *caster == owner)).count();
        assert_eq!(cancelled, 2);
        assert_eq!(stopped.sidecar(owner.entity).unwrap().cast, CastState::Idle);
        assert!(stopped
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
        stopped.validate_runtime_v5().unwrap();
        // The retained resurrection binding has become living, a legal stale target at admission.
        let launch = zone.phase4.as_ref().unwrap().canonical.item_casts[&main_id]
            .cast
            .launch_at;
        while zone.next_tick < launch {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(tick.events.iter().filter(|event| matches!(event, ZoneEvent::SkillCastCancelled { cast_id, .. } if *cast_id == main_id)).count(), 1);
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
        assert_eq!(zone.sidecar(owner.entity).unwrap().cast, CastState::Idle);
        zone.validate_runtime_v5().unwrap();
    }
    fn use_fixture(
        templates: &[u32],
    ) -> (
        ZoneState,
        TargetBinding,
        TargetBinding,
        Vec<crate::domain::zone::ItemInstanceId>,
    ) {
        use crate::domain::zone::ItemInstanceId;
        let (mut zone, owner, npc) = equipment_fixture();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        let seed = player.inventory[&ItemInstanceId::new(900).unwrap()].clone();
        let mut ids = Vec::new();
        for (offset, template) in templates.iter().enumerate() {
            let id =
                ItemInstanceId::new(901_u64.checked_add(u64::try_from(offset).unwrap()).unwrap())
                    .unwrap();
            let mut item = seed.clone();
            item.id = id;
            item.template_id = *template;
            item.count = if rules.parts().items[template].stackable {
                3
            } else {
                1
            };
            player.inventory.insert(id, item);
            ids.push(id);
        }
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        zone.validate_runtime_v5().unwrap();
        (zone, owner, npc, ids)
    }

    fn use_input(
        zone: &ZoneState,
        owner: TargetBinding,
        key: u128,
        seq: u32,
        instance: crate::domain::zone::ItemInstanceId,
        target: Option<TargetBinding>,
    ) -> ZoneInput {
        ZoneInput::session(
            owner.entity,
            SessionGeneration(1),
            seq,
            ZoneCommand::Item(Box::new(item_intent(
                zone,
                owner.entity,
                key,
                crate::domain::zone::ItemCommandKind::Use { instance, target },
            ))),
        )
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Actual Use entry proves payment/reuse/frozen identity and both warm/cold paths.
    fn v6_use_cp_pays_once_reuses_freezes_sequence_and_cold_replays() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        let (mut zone, owner, _, ids) = use_fixture(&[5591, 5592]);
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            unreachable!()
        };
        player.class_state.cp = 0;
        let learned = player.class_state.learned_skills.clone();
        let before =
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory_revision;
        let cursor = zone.rng.get_word_pos();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = zone.draft(vec![
            use_input(&zone, owner, 910, 10, ids[0], None),
            use_input(&zone, owner, 910, 11, ids[0], None),
            use_input(&zone, owner, 911, 12, ids[0], None),
            use_input(&zone, owner, 912, 13, ids[1], None),
        ]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(applied.dispositions.len(), 1, "{:?}", applied.dispositions);
        assert_eq!(applied.dispositions[0].reason, RejectReason::ItemReuse);
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.receipts.len(), 2);
        assert_eq!(
            projection
                .ledger
                .iter()
                .filter(|row| row.operation == ItemOperation::Consume)
                .count(),
            2
        );
        assert_eq!(projection.checkpoint.inventory_revision, before.checked_add(1).unwrap());
        assert_eq!(projection.receipts[0].result.accepted_sequence, Some(10));
        let results: Vec<_> = applied
            .events
            .iter()
            .filter_map(|event| {
                if let ZoneEvent::ItemResult(result) = event {
                    Some(result)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], results[1]);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.item_casts.len(), 2);
        for id in &ids {
            assert_eq!(
                zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[id].count,
                2
            );
        }
        assert_eq!(zone.rng.get_word_pos(), cursor);
        zone.validate_runtime_v5().unwrap();
        let mut restored =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(tick, restored.run_tick(restored.draft(Vec::new())).unwrap());
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&owner.entity].combat.as_ref().unwrap().role
        else {
            unreachable!()
        };
        assert_eq!(player.class_state.cp, zone.effective(owner.entity).unwrap().max_cp.min(250));
        assert_eq!(player.class_state.learned_skills, learned);
        assert_eq!(zone.rng.get_word_pos(), cursor);
        assert_eq!(
            tick.events
                .iter()
                .filter(|event| matches!(event, ZoneEvent::SkillCastResult { .. }))
                .count(),
            2
        );
        assert!(!tick
            .events
            .iter()
            .any(|event| matches!(event, ZoneEvent::ItemCheckpoint(_))));
    }

    #[test]
    fn v6_use_cocktail_fallback_is_one_payment_for_all_source_skills_and_stop_order() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        let (mut zone, owner, _, ids) = use_fixture(&[20394]);
        let cursor = zone.rng.get_word_pos();
        let draft = zone.draft(vec![
            use_input(&zone, owner, 910, 1, ids[0], None),
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                2,
                ZoneCommand::StopMove {
                    entity: owner.entity,
                },
            ),
            use_input(&zone, owner, 911, 3, ids[0], None),
        ]);
        let applied = zone.run_tick(draft).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.receipts.len(), 1);
        assert_eq!(projection.receipts[0].request_id, Uuid::from_u128(911));
        assert_eq!(projection.receipts[0].result.accepted_sequence, Some(3));
        assert_eq!(projection.ledger.len(), 1);
        assert_eq!(projection.ledger[0].operation, ItemOperation::Consume);
        assert_eq!(projection.ledger[0].count_delta, -1);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.item_casts.len(), 8);
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            2
        );
        assert_eq!(zone.rng.get_word_pos(), cursor);
        zone.validate_runtime_v5().unwrap();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        let applied = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(applied, cold.run_tick(cold.draft(Vec::new())).unwrap());
        assert_eq!(
            applied
                .events
                .iter()
                .filter(|event| matches!(event, ZoneEvent::SkillCastResult { .. }))
                .count(),
            8
        );
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // A real resurrection item holds the main lane while CP resolves separately.
    fn v6_use_resurrection_and_cp_share_lifecycle_and_do_not_refund_decline() {
        let (mut zone, owner, npc, ids) = use_fixture(&[737, 5591]);
        let dead = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .players
            .keys()
            .copied()
            .find(|id| *id != owner.entity)
            .unwrap();
        let target = zone.binding(dead).unwrap();
        zone.apply_damage_v5(
            zone.next_tick,
            DamageRequest {
                source: zone.damage_source_v5(npc).unwrap(),
                target,
                cause: DamageCause::Direct,
                rolled_damage: u32::MAX,
            },
            &mut Vec::new(),
        )
        .unwrap();
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            unreachable!()
        };
        player.class_state.cp = 0;
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let applied = zone
            .run_tick(zone.draft(vec![
                ZoneInput::session(
                    owner.entity,
                    SessionGeneration(1),
                    1,
                    ZoneCommand::SetTarget {
                        entity: owner.entity,
                        target: Some(dead),
                    },
                ),
                use_input(&zone, owner, 920, 2, ids[0], None),
                ZoneInput::session(
                    owner.entity,
                    SessionGeneration(1),
                    3,
                    ZoneCommand::SetTarget {
                        entity: owner.entity,
                        target: Some(owner.entity),
                    },
                ),
                use_input(&zone, owner, 921, 4, ids[1], None),
            ]))
            .unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let CastState::Casting(main) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
            unreachable!()
        };
        assert_eq!(main.skill.id.0, 2014);
        assert_eq!(main.target, Some(target));
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.item_casts.len(), 2);
        let cursor = zone.rng.get_word_pos();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        let mut offered = false;
        while zone.next_tick <= main.complete_at {
            let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
            assert_eq!(tick, cold.run_tick(cold.draft(Vec::new())).unwrap());
            offered |= tick.events.iter().any(|event| {
                matches!(
                    event,
                    ZoneEvent::ResurrectOffer {
                        state: crate::domain::zone::ResurrectOfferState::Offered,
                        ..
                    }
                )
            });
        }
        assert!(offered);
        assert_eq!(zone.rng.get_word_pos(), cursor);
        let offer = zone
            .sidecar(dead)
            .unwrap()
            .player
            .as_ref()
            .unwrap()
            .resurrect_offer
            .as_ref()
            .unwrap()
            .id;
        let tick = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                dead,
                SessionGeneration(1),
                3,
                ZoneCommand::AcceptResurrect {
                    entity: dead,
                    offer_id: offer,
                    accept: false,
                },
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty());
        assert!(zone.entities[&dead].targeting.dead);
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            2
        );
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
    }
    #[test]
    fn v6_escape_scrolls_keep_source_timing_life_and_cold_replay() {
        // L2Character.java:1636/1643,4581: clamp launch then add the 400ms impact stage.
        for (template, duration) in [(736, 200), (1538, 4)] {
            let (mut zone, owner, _, ids) = use_fixture(&[template]);
            let town = Vec2Fixed::from_tiles(126, 126);
            zone = zone.with_safe_point(town);
            let cursor = zone.rng.get_word_pos();
            let start = zone.next_tick;
            let applied = zone
                .run_tick(zone.draft(vec![use_input(&zone, owner, 940, 1, ids[0], None)]))
                .unwrap();
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
            let CastState::Casting(cast) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
                unreachable!()
            };
            assert_eq!(cast.complete_at.0 - start.0, duration);
            assert_eq!(
                zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]]
                    .count,
                2
            );
            let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
            let mut cold =
                ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
            let mut relocated = false;
            while zone.next_tick <= cast.complete_at {
                let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
                assert_eq!(tick, cold.run_tick(cold.draft(Vec::new())).unwrap());
                assert!(!tick
                    .events
                    .iter()
                    .any(|e| matches!(e, ZoneEvent::EntityRespawned { .. })));
                relocated |= tick.events.iter().any(|e| matches!(e, ZoneEvent::EntityMove { entity, pos, .. } if *entity == owner.entity && *pos == town));
                if tick.tick < cast.complete_at {
                    assert_ne!(zone.entities[&owner.entity].pos, town);
                }
            }
            assert!(relocated);
            assert_eq!(zone.binding(owner.entity), Some(owner));
            assert_eq!(zone.entities[&owner.entity].pos, town);
            assert!(!zone.entities[&owner.entity].targeting.dead);
            assert_eq!(zone.rng.get_word_pos(), cursor);
            assert!(zone
                .phase4
                .as_ref()
                .unwrap()
                .canonical
                .item_casts
                .is_empty());
            zone.validate_runtime_v5().unwrap();
        }
    }

    #[test]
    fn v6_escape_refuses_without_destination_and_damage_cancel_keeps_payment() {
        let (mut zone, owner, _, ids) = use_fixture(&[736]);
        zone.safe_point = None;
        let cursor = zone.rng.get_word_pos();
        let refused = zone
            .run_tick(zone.draft(vec![use_input(&zone, owner, 950, 1, ids[0], None)]))
            .unwrap();
        assert_eq!(refused.dispositions[0].reason, RejectReason::NotPermitted);
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            3
        );
        assert_eq!(zone.rng.get_word_pos(), cursor);
        zone = zone.with_safe_point(Vec2Fixed::from_tiles(126, 126));
        let accepted = zone
            .run_tick(zone.draft(vec![use_input(&zone, owner, 951, 2, ids[0], None)]))
            .unwrap();
        assert!(accepted.dispositions.is_empty());
        let mut trace = Vec::new();
        let mut events = Vec::new();
        zone.attack_break_v5(
            zone.next_tick,
            owner,
            1,
            &mut events,
            &mut scripted(&[(DamageRollSite::AttackBreak, 100, 0)], &mut trace),
        )
        .unwrap();
        assert_eq!(trace.len(), 1);
        assert_eq!(zone.sidecar(owner.entity).unwrap().cast, CastState::Idle);
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
        assert!(events.iter().any(|e| matches!(
            e,
            ZoneEvent::SkillCastCancelled {
                reason: CastCancelReason::Damage,
                ..
            }
        )));
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            2
        );
        assert_ne!(zone.entities[&owner.entity].pos, zone.safe_point.unwrap());
    }
    #[test]
    fn v6_hp_potion_uses_source_cost_alias_and_plain_hp_without_heal_rng() {
        // 21040's source cost14701 is NPC-only/non-storable. It must refuse, not
        // silently substitute the selected item; accepted D54 defers it before payment.
        let (mut missing, owner, _, ids) = use_fixture(&[21040]);
        let cursor = missing.rng.get_word_pos();
        let rejected = missing
            .run_tick(missing.draft(vec![use_input(&missing, owner, 960, 1, ids[0], None)]))
            .unwrap();
        assert_eq!(rejected.dispositions[0].reason, RejectReason::ItemCapabilityDeferred);
        assert_eq!(missing.sidecar(owner.entity).unwrap().cast, CastState::Idle);
        assert_eq!(
            missing.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]]
                .count,
            3
        );
        assert_eq!(missing.rng.get_word_pos(), cursor);
        // Potion of a Hero uses both the source Hp500 and Mp200 instant handlers.
        let (mut zone, owner, _, ids) = use_fixture(&[21038]);
        set_resources(&mut zone, owner, 1, 5);
        let cursor = zone.rng.get_word_pos();
        let started = zone
            .run_tick(zone.draft(vec![use_input(&zone, owner, 960, 1, ids[0], None)]))
            .unwrap();
        assert!(started.dispositions.is_empty(), "{:?}", started.dispositions);
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            2
        );
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(tick, cold.run_tick(cold.draft(Vec::new())).unwrap());
        let combat = zone.entities[&owner.entity].combat.as_ref().unwrap();
        assert_eq!(combat.hp, combat.sheet.max_hp().min(501));
        assert_eq!(combat.mp, combat.sheet.max_mp().min(205));
        assert_eq!(zone.rng.get_word_pos(), cursor);
        assert!(!tick
            .events
            .iter()
            .any(|event| matches!(event, ZoneEvent::ItemCheckpoint(_))));
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
    }
    #[test]
    #[allow(clippy::too_many_lines)] // Fixture-seeded learned rank, real skill/cost definitions and critical projection.
    fn v6_learned_item_cost_is_atomic_paid_once_and_not_refunded_on_cancel() {
        use crate::domain::subclass::LearnedSkill;
        use crate::domain::zone::skill::SkillId;
        let (mut zone, owner, _, ids) = use_fixture(&[3031]);
        let xp_at_60 = zone.rules.as_ref().unwrap().xp_to_level(60).unwrap();
        let base_stats = zone
            .classes
            .as_ref()
            .unwrap()
            .get(ClassId(10))
            .unwrap()
            .base_stats;
        let combat = zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap();
        let CombatRole::Player {
            xp,
            progression: Some(player),
            ..
        } = &mut combat.role
        else {
            unreachable!()
        };
        *xp = xp_at_60;
        player.class_state.current_class_id = ClassId(17);
        // A fixture-seeded Prophet with both required historical transfer receipts.
        for (target_class, level) in [(15, 20), (17, 40)] {
            player.class_state.successful_transfer_receipts.push(
                crate::domain::character_progression::SuccessfulTransferReceipt {
                    key: Uuid::from_u128(u128::from(target_class)),
                    target_class_id: ClassId(target_class),
                    result: crate::domain::character_progression::FrozenTransferResult {
                        character_id: crate::domain::CharacterId::from_uuid(owner.entity.as_uuid()),
                        identity: player.identity.clone(),
                        name: crate::domain::CharacterName::new("First").unwrap(),
                        current_class_id: ClassId(target_class),
                        level,
                        xp: 0,
                        sp: 0,
                        stats: base_stats,
                        position_millitiles: [10000, 10000],
                        hp: 1,
                        mp: 1,
                        cp: 0,
                        max_hp: 1,
                        max_mp: 1,
                        max_cp: 0,
                        token_tier_1_count: 0,
                        token_tier_2_count: 0,
                        granted_skill_keys: Vec::new(),
                    },
                },
            );
        }
        player.class_state.merge_learned_skills(vec![LearnedSkill {
            key: "l2.skill.1388".into(),
            level: 1,
        }]);
        assert!(player
            .class_state
            .validate_for_phase3(
                zone.classes.as_ref().unwrap(),
                &player.identity,
                crate::domain::CharacterId::from_uuid(owner.entity.as_uuid())
            )
            .is_err());
        player
            .class_state
            .validate_for_skill_rules(
                zone.classes.as_ref().unwrap(),
                &player.identity,
                crate::domain::CharacterId::from_uuid(owner.entity.as_uuid()),
                &zone.phase3.as_ref().unwrap().canonical.rules,
            )
            .unwrap();
        zone.relevel(owner.entity).unwrap();
        zone.sync_passives_v5(owner.entity).unwrap();
        zone.rebuild_v5(owner.entity).unwrap();
        let max = zone.effective(owner.entity).unwrap();
        set_resources(&mut zone, owner, max.physical.max_hp(), max.max_cp);
        zone.entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .mp = max.physical.max_mp();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let input = |seq| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                seq,
                ZoneCommand::CastSkill {
                    entity: owner.entity,
                    skill_id: SkillId(1388),
                    target: Some(owner.entity),
                },
            )
        };
        let cursor = zone.rng.get_word_pos();
        let mut missing = zone.clone();
        missing
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .remove(&ids[0]);
        missing.rebuild_v5(owner.entity).unwrap();
        let mana = missing.entities[&owner.entity].combat.as_ref().unwrap().mp;
        let refused = missing.run_tick(missing.draft(vec![input(1)])).unwrap();
        assert_eq!(refused.dispositions[0].reason, RejectReason::InsufficientItems);
        assert_eq!(missing.entities[&owner.entity].combat.as_ref().unwrap().mp, mana);
        assert_eq!(missing.sidecar(owner.entity).unwrap().cast, CastState::Idle);
        assert_eq!(missing.rng.get_word_pos(), cursor);
        let snapshot = zone.snapshot();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = zone.draft(vec![input(1)]);
        let started = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(started, cold.run_tick(draft).unwrap());
        assert!(started.dispositions.is_empty(), "{:?}", started.dispositions);
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_CAST_COST_EVIDENCE_RECORDING") {
            use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: snapshot.seed.zone,
                    epoch: snapshot.seed.epoch,
                    records: 1,
                    last_tick: Some(started.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records: vec![AppliedTickRecord::from_applied(
                    snapshot.seed.zone,
                    &started,
                )],
                snapshot,
            };
            let bytes = recording.to_bytes().unwrap();
            let decoded = crate::infrastructure::eventlog::Recording::from_bytes(&bytes).unwrap();
            assert!(decoded.records[0].reproduced_by(&recording.records[0]));
            std::fs::write(path, bytes).unwrap();
        }

        let projection = started
            .events
            .iter()
            .find_map(|e| {
                if let ZoneEvent::ItemCheckpoint(p) = e {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(projection.receipts.is_empty());
        assert_eq!(projection.ledger.len(), 1);
        assert_eq!(projection.ledger[0].count_delta, -1);
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            2
        );
        let CastState::Casting(cast) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
            unreachable!()
        };
        let mut canceled = zone.clone();
        let stopped = canceled
            .run_tick(canceled.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                2,
                ZoneCommand::StopMove {
                    entity: owner.entity,
                },
            )]))
            .unwrap();
        assert!(stopped.dispositions.is_empty());
        assert_eq!(canceled.sidecar(owner.entity).unwrap().cast, CastState::Idle);
        assert_eq!(
            canceled.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]]
                .count,
            2
        );
        let mut restored =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        while zone.next_tick <= cast.complete_at {
            let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
            assert_eq!(tick, restored.run_tick(restored.draft(Vec::new())).unwrap());
            assert!(!tick
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(_))));
        }
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&ids[0]].count,
            2
        );
        assert_eq!(zone.rng.get_word_pos(), cursor);
        assert_eq!(zone.sidecar(owner.entity).unwrap().cast, CastState::Idle);
    }
    #[test]
    fn v6_item_evidence_captures_enchant_inputs_without_changing_replay() {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        use crate::domain::zone::evidence::ItemEvidenceData;
        let (mut live, owner, target, scroll) =
            enchant_fixture(956, crate::domain::zone::EquipSlot::Chest, 0);
        let snapshot = live.snapshot();
        let mut observed = live.clone();
        observed.enable_items_replay_evidence().unwrap();
        let draft = live.draft(
            (0_u32..2)
                .map(|index| {
                    ZoneInput::session(
                        owner.entity,
                        SessionGeneration(1),
                        index + 1,
                        ZoneCommand::Item(Box::new(item_intent(
                            &live,
                            owner.entity,
                            950 + u128::from(index),
                            crate::domain::zone::ItemCommandKind::Enchant {
                                target,
                                scroll,
                                support: None,
                            },
                        ))),
                    )
                })
                .collect(),
        );
        let applied = live.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty());
        assert_eq!(applied, observed.run_tick(draft).unwrap());
        let facts = observed.take_items_replay_evidence();
        assert_eq!(
            facts
                .iter()
                .filter(|f| matches!(f.fact, ItemEvidenceData::Baseline(_)))
                .count(),
            2
        );
        assert_eq!(
            facts
                .iter()
                .filter(|f| matches!(f.fact, ItemEvidenceData::Delta(_)))
                .count(),
            4
        );
        let draws: Vec<_> = facts
            .iter()
            .filter_map(|f| match &f.fact {
                ItemEvidenceData::Rng {
                    word_before,
                    word_after,
                    bound,
                    value,
                    callsite,
                    ..
                } => Some((*word_before, *word_after, *bound, *value, callsite.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(draws.len(), 2);
        assert_eq!(draws[1].0, draws[0].1);
        assert_eq!(draws[1].1, draws[1].0 + 1);
        assert_eq!(draws[0].2, 100_000);
        assert_eq!(draws[0].4, "enchant");
        assert_eq!(draws[0].1, draws[0].0 + 1);
        let formulas: Vec<_> = facts
            .iter()
            .filter_map(|f| match &f.fact {
                ItemEvidenceData::Formula(data) => Some(data),
                _ => None,
            })
            .collect();
        assert_eq!(formulas.len(), 4);
        assert_eq!(formulas[0]["actual"], 100_000);
        assert_eq!(formulas[1]["actual"]["enchant"], 1);
        assert_eq!(formulas[1]["operation"]["target_before"]["enchant"], 0);
        assert_eq!(formulas[1]["operation"]["scroll_before"]["count"], 3);
        assert_eq!(formulas[3]["actual"]["enchant"], 2);
        assert_eq!(formulas[3]["operation"]["target_before"]["enchant"], 1);
        assert_eq!(formulas[3]["operation"]["scroll_before"]["count"], 2);
        assert_ne!(formulas[1]["operation"]["receipt"], formulas[3]["operation"]["receipt"]);
        assert_eq!(live.snapshot(), observed.snapshot());
        // Manual CLI/consumer probe uses this exact generated recording, never a synthetic verdict.
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_ITEM_EVIDENCE_RECORDING") {
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: snapshot.seed.zone,
                    epoch: snapshot.seed.epoch,
                    records: 1,
                    last_tick: Some(applied.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records: vec![AppliedTickRecord::from_applied(
                    snapshot.seed.zone,
                    &applied,
                )],
                snapshot,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }
    }
    #[allow(clippy::arithmetic_side_effects)]
    fn exchange_fixture(
    ) -> (ZoneState, TargetBinding, TargetBinding, crate::domain::zone::ItemInstanceId) {
        use crate::domain::zone::{EquipSlot, ItemInstanceId, ItemLocation};
        let (mut zone, owner, _) = equipment_fixture();
        let items = zone.phase4.as_mut().unwrap();
        let recipe = items.rules.parts().sa_recipes[&4_316_442_132_483].clone();
        let player = items.canonical.players.get_mut(&owner.entity).unwrap();
        let target = player.paperdoll[&EquipSlot::Rhand];
        let mut seed = player
            .inventory
            .remove(&ItemInstanceId::new(900).unwrap())
            .unwrap();
        for (index, ingredient) in recipe
            .ingredients
            .iter()
            .filter(|i| i.template_id != recipe.base_template)
            .enumerate()
        {
            seed.id = ItemInstanceId::new(901 + u64::try_from(index).unwrap()).unwrap();
            seed.template_id = ingredient.template_id;
            seed.count = if items.rules.parts().items[&ingredient.template_id].stackable {
                ingredient.count * 2
            } else {
                ingredient.count
            };
            player.inventory.insert(seed.id, seed.clone());
            if !items.rules.parts().items[&ingredient.template_id].stackable {
                seed.id = ItemInstanceId::new(911 + u64::try_from(index).unwrap()).unwrap();
                player.inventory.insert(seed.id, seed.clone());
            }
        }
        player.inventory.get_mut(&target).unwrap().augment_id = Some((2 << 16) | 1);
        let mut second = player.inventory[&target].clone();
        second.id = ItemInstanceId::new(920).unwrap();
        second.location = ItemLocation::Inventory;
        second.slot = None;
        player.inventory.insert(second.id, second);
        crate::domain::zone::item_sources::reconcile_sources(
            player,
            &items.rules,
            &mut zone
                .phase3
                .as_mut()
                .unwrap()
                .canonical
                .next_install_sequence,
        )
        .unwrap();
        let from = zone.entities[&owner.entity].pos;
        let to = Vec2Fixed::from_tiles(126, 126);
        zone.entities.get_mut(&owner.entity).unwrap().pos = to;
        zone.aoi.relocate(owner.entity, from, to);
        zone.rebuild_v5(owner.entity).unwrap();
        let applied = zone
            .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
                name: "blacksmith".into(),
                pos: Vec2Fixed::from_tiles(124, 128),
                speed: Speed::from_milli_tiles_per_tick(0),
                combat: None,
            })]))
            .unwrap();
        assert!(applied.dispositions.is_empty());
        let npc = TargetBinding {
            entity: zone
                .entities
                .values()
                .find(|e| e.name == "blacksmith")
                .unwrap()
                .id,
            incarnation: 0,
        };
        zone.validate_runtime_v5().unwrap();
        (zone, owner, npc, target)
    }

    #[test]
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    fn v6_exchange_two_receipts_aggregate_shared_ingredients_and_cold_replay() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (mut zone, owner, npc, target) = exchange_fixture();
        let old = zone.phase4.as_ref().unwrap().canonical.clone();
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let first = item_intent(
            &zone,
            owner.entity,
            701,
            K::ExchangeSpecialAbility {
                npc,
                target,
                recipe: 4_316_442_132_483,
            },
        );
        let second = item_intent(
            &zone,
            owner.entity,
            702,
            K::ExchangeSpecialAbility {
                npc,
                target: ItemInstanceId::new(920).unwrap(),
                recipe: 4_316_442_132_483,
            },
        );
        let draft = zone.draft(
            [first.clone(), first, second]
                .into_iter()
                .enumerate()
                .map(|(index, command)| {
                    ZoneInput::session(
                        owner.entity,
                        SessionGeneration(1),
                        u32::try_from(index + 1).unwrap(),
                        ZoneCommand::Item(Box::new(command)),
                    )
                })
                .collect(),
        );
        zone.enable_items_replay_evidence().unwrap();
        let cursor = zone.rng.get_word_pos();
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(cursor, zone.rng.get_word_pos());
        let evidence = zone.take_items_replay_evidence();
        let facts = evidence
            .iter()
            .filter_map(|fact| {
                if let crate::domain::zone::evidence::ItemEvidenceData::Formula(value) = &fact.fact
                {
                    Some(value)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0]["formula"], "sa_exchange");
        assert_eq!(
            facts[0]["inputs"]["ingredients_after"],
            facts[1]["inputs"]["ingredients_before"]
        );
        assert_eq!(facts[1]["inputs"]["ingredients_after"]["57"], 0);
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_SA_FACTS") {
            std::fs::write(path, serde_json::to_vec(&facts).unwrap()).unwrap();
        }
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.exchanges.len(), 2);
        assert_eq!(projection.receipts.len(), 2);
        assert_eq!(
            applied
                .events
                .iter()
                .filter(|e| matches!(e, ZoneEvent::ItemResult(_)))
                .count(),
            3
        );
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory_revision, old.players[&owner.entity].inventory_revision + 1);
        assert!(!player.paperdoll.contains_key(&EquipSlot::Rhand));
        for (index, audit) in projection.exchanges.iter().enumerate() {
            assert_eq!(
                audit.after.id.get(),
                old.allocator.bounds().1 + u64::try_from(index).unwrap()
            );
            assert_eq!(
                (audit.before.template_id, audit.after.template_id, audit.after.enchant),
                (71, 4711, 4)
            );
            assert!(audit.after.slot.is_none());
            assert_eq!(audit.after.augment_id, Some((2 << 16) | 1));
            assert!(audit
                .rows
                .iter()
                .all(|row| row.receipt == Some(audit.receipt)));
            assert_eq!(
                audit
                    .rows
                    .iter()
                    .map(|row| row.template().unwrap())
                    .collect::<Vec<_>>(),
                vec![71, 4634, 2131, 57, 4711]
            );
        }
        for id in [902, 903] {
            let id = ItemInstanceId::new(id).unwrap();
            let rows = projection
                .ledger
                .iter()
                .filter(|row| row.instance == id && row.operation == ItemOperation::Consume)
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].causes.len(), 2);
            assert_eq!(
                rows[0].receipt_refs,
                projection
                    .exchanges
                    .iter()
                    .map(|audit| audit.receipt)
                    .collect::<Vec<_>>()
            );
            for corrupt in 0..5 {
                let mut invalid = projection.as_ref().clone();
                let aggregate = invalid
                    .ledger
                    .iter_mut()
                    .find(|row| row.instance == id && row.operation == ItemOperation::Consume)
                    .unwrap();
                match corrupt {
                    0 => aggregate.receipt_refs.clear(),
                    1 => aggregate.receipt_refs.reverse(),
                    2 => aggregate.receipt_refs.push(aggregate.receipt_refs[0]),
                    3 => aggregate.receipt_refs[0].key = Uuid::from_u128(99999),
                    _ => {
                        aggregate.receipt = aggregate.receipt_refs.first().copied();
                        aggregate.receipt_refs.clear();
                    },
                }
                assert!(invalid
                    .validate_structure(
                        zone.sidecar(owner.entity)
                            .unwrap()
                            .modifier_install_order
                            .len()
                    )
                    .is_err());
            }
            assert_eq!(
                rows[0].count_delta,
                -i64::try_from(old.players[&owner.entity].inventory[&id].count).unwrap()
            );
            assert!(rows[0].after.is_none());
        }
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
        assert_eq!(
            zone.snapshot(),
            ZoneState::from_snapshot_with_items(
                zone.snapshot(),
                Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules))
            )
            .unwrap()
            .snapshot()
        );
        for case in 0..8 {
            let mut invalid = projection.as_ref().clone();
            match case {
                0 => invalid.exchanges.clear(),
                1 => invalid.exchanges[0].rows[1].receipt = None,
                2 => invalid.exchanges[0].rows[1].count_delta -= 1,
                3 => invalid.exchanges[0].after.enchant += 1,
                4 => invalid.exchanges[0].after.id = target,
                5 => invalid.exchanges[0].recipe += 1,
                6 => invalid.exchanges[0].rows.swap(0, 1),
                _ => invalid.exchanges.push(invalid.exchanges[0].clone()),
            }
            assert!(
                invalid
                    .validate_structure(
                        zone.phase3
                            .as_ref()
                            .unwrap()
                            .canonical
                            .checkpoint(owner.entity, zone.next_tick)
                            .unwrap()
                            .modifier_install_order
                            .len()
                    )
                    .is_err(),
                "case {case}"
            );
        }
    }

    #[test]
    fn v6_exchange_invalid_guard_cost_recipe_and_capacity_do_not_mutate() {
        use crate::domain::zone::{ItemCommandKind as K, ItemInstanceId};
        let (zone, owner, npc, target) = exchange_fixture();
        for case in 0..7 {
            let mut zone = zone.clone();
            let mut npc = npc;
            let mut recipe = 4_316_442_132_483;
            match case {
                0 => npc.incarnation = 1,
                1 => {
                    zone.entities.get_mut(&owner.entity).unwrap().pos =
                        Vec2Fixed::from_tiles(127, 129);
                },
                2 => {
                    zone.phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap()
                        .inventory
                        .remove(&ItemInstanceId::new(902).unwrap());
                },
                3 => recipe = 4_316_442_132_481,
                4 => {
                    zone.phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap()
                        .max_load = 0;
                },
                5 => {
                    let (start, _, end) =
                        zone.phase4.as_ref().unwrap().canonical.allocator.bounds();
                    zone.phase4.as_mut().unwrap().canonical.allocator =
                        crate::domain::zone::ItemInstanceRange::new(start, end, end).unwrap();
                },
                _ => zone.entities.get_mut(&npc.entity).unwrap().targeting.dead = true,
            }
            let snapshot = zone.snapshot();
            let command = item_intent(
                &zone,
                owner.entity,
                704,
                K::ExchangeSpecialAbility {
                    npc,
                    target,
                    recipe,
                },
            );
            let mut events = Vec::new();
            assert!(
                zone.item_command_v6(zone.next_tick, &command, &mut events, &mut BTreeMap::new())
                    .is_err(),
                "case {case}"
            );
            assert_eq!(zone.snapshot(), snapshot, "case {case}");
            assert!(events.is_empty());
        }
    }
    #[test]
    fn v6_fractional_hp_cold_replay_regen_and_last_fraction_death() {
        use crate::domain::zone::item_combat::half_hp;
        let (mut base, source, npc) = equipment_fixture();
        base.run_tick(base.draft(Vec::new())).unwrap();
        for original_hp in [1, 2, 3] {
            let mut zone = base.clone();
            let (whole, remainder) = half_hp(original_hp, 0).unwrap();
            set_resources(&mut zone, npc, whole, 0);
            zone.sidecar_mut(npc.entity).unwrap().fractions.hp = remainder;
            let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
            let mut cold =
                ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules)))
                    .unwrap();
            assert!(!cold.entities[&npc.entity].targeting.dead);
            if original_hp == 1 {
                for dead_flag in [false, true] {
                    let mut invalid = zone.snapshot();
                    if dead_flag {
                        invalid
                            .entities
                            .iter_mut()
                            .find(|e| e.id == npc.entity)
                            .unwrap()
                            .targeting
                            .dead = true;
                    } else {
                        invalid
                            .phase3
                            .as_mut()
                            .unwrap()
                            .entities
                            .get_mut(&npc.entity)
                            .unwrap()
                            .fractions
                            .hp = 0;
                    }
                    assert!(ZoneState::from_snapshot_with_items(invalid, Some(Arc::clone(&rules)))
                        .is_err());
                }
            }
            let due = zone.sidecar(npc.entity).unwrap().regen.next_at.unwrap();
            while zone.next_tick <= due {
                let draft = zone.draft(Vec::new());
                let actual = zone.run_tick(draft.clone()).unwrap();
                assert_eq!(actual, cold.run_tick(draft).unwrap());
            }
            assert!(!zone.entities[&npc.entity].targeting.dead);
            assert!(zone.entities[&npc.entity].combat.as_ref().unwrap().hp > whole);
            assert_eq!(zone.snapshot(), cold.snapshot());
        }
        let mut zone = base;
        set_resources(&mut zone, npc, 0, 0);
        zone.sidecar_mut(npc.entity).unwrap().fractions.hp = Q / 2;
        let mut events = Vec::new();
        for damage in [0, 1] {
            let result = zone
                .apply_damage_rolled_v5(
                    zone.next_tick,
                    DamageRequest {
                        source: zone.damage_source_v5(source).unwrap(),
                        target: npc,
                        cause: DamageCause::Direct,
                        rolled_damage: damage,
                    },
                    &mut events,
                    &mut scripted(&[], &mut Vec::new()),
                )
                .unwrap();
            assert_eq!(result.lethal, damage == 1);
            assert_eq!(zone.entities[&npc.entity].targeting.dead, damage == 1);
        }
        assert_eq!(zone.sidecar(npc.entity).unwrap().fractions, ResourceFractions::default());
        assert!(zone.sidecar(npc.entity).unwrap().regen.next_at.is_none());
        assert_eq!(
            events
                .iter()
                .filter(
                    |e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == npc.entity)
                )
                .count(),
            1
        );
    }

    #[test]
    fn v6_fractional_hp_paused_admission_preserves_alive_zero_display() {
        let (mut zone, owner, _) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        set_resources(&mut zone, owner, 0, 0);
        zone.sidecar_mut(owner.entity).unwrap().fractions.hp = Q / 2;
        let phase = &zone.phase3.as_ref().unwrap().canonical;
        let checkpoint = phase.checkpoint(owner.entity, zone.next_tick).unwrap();
        let items = &zone.phase4.as_ref().unwrap().canonical;
        let item_checkpoint = items
            .checkpoint(owner.entity, zone.next_tick, phase)
            .unwrap();
        let entity = &zone.entities[&owner.entity];
        let combat = entity.combat.as_ref().unwrap();
        let CombatRole::Player {
            progression: Some(progression),
            class,
            xp,
        } = &combat.role
        else {
            panic!("player")
        };
        let load = PlayerLoad {
            phase4: Some(crate::domain::zone::Phase4PlayerLoad {
                checkpoint: Some(item_checkpoint),
                pending: Vec::new(),
            }),
            phase3: Some(checkpoint),
            progression: Some(progression.clone()),
            checkpoint_revision: Some(1),
            class: class.clone(),
            xp: *xp,
            level: combat.sheet.level(),
            hp: Some(0),
            mp: Some(combat.mp),
            alive: true,
        };
        let spawn = ZoneCommand::SpawnPlayer {
            entity: owner.entity,
            name: entity.name.clone(),
            pos: entity.pos,
            speed: Speed::DEFAULT,
            generation: SessionGeneration(2),
            load: Some(Box::new(load)),
        };
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = zone.draft(vec![
            ZoneInput::system(ZoneCommand::Despawn {
                entity: owner.entity,
            }),
            ZoneInput::system(spawn),
        ]);
        let actual = zone.run_tick(draft.clone()).unwrap();
        assert!(actual.dispositions.is_empty(), "{:?}", actual.dispositions);
        assert_eq!(actual, cold.run_tick(draft).unwrap());
        assert!(!zone.entities[&owner.entity].targeting.dead);
        assert_eq!(zone.entities[&owner.entity].combat.as_ref().unwrap().hp, 0);
        assert_eq!(zone.sidecar(owner.entity).unwrap().fractions.hp, Q / 2);
        assert!(zone.sidecar(owner.entity).unwrap().regen.next_at.is_some());
        assert!(actual.events.iter().any(|event| matches!(event, ZoneEvent::EntitySpawn { entity, combat: Some(view), .. } if *entity == owner.entity && view.hp == 0 && !view.dead)));
        assert_eq!(
            zone.snapshot(),
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules))
                .unwrap()
                .snapshot()
        );
    }
    #[test]
    #[allow(clippy::too_many_lines)] // One source fixture covers Q boundaries, both resource setters and skipped draws.
    fn v6_lethal_callers_use_q_setters_and_conditional_draws_without_death_hooks() {
        use crate::domain::zone::skill::SkillId;
        use DamageRollSite as R;
        let (mut original, owner, npc) = equipment_fixture();
        original.run_tick(original.draft(Vec::new())).unwrap();
        // Actual pinned handler descriptors; these direct caller checks do not promote
        // the still-deferred learned-skill capability or bypass CastSkill admission.
        let skill = original
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(SkillId(30))
            .unwrap()
            .clone();
        let handler = skill.effects.iter().find(|h| h.name == "Lethal").unwrap();
        for (whole, remainder, expected) in [
            (1, 0, (0, 500_000)),
            (2, 0, (1, 0)),
            (3, 0, (1, 500_000)),
            (0, 1, (0, 1)),
            (0, 2, (0, 1)),
        ] {
            let mut zone = original.clone();
            set_resources(&mut zone, npc, whole, 0);
            zone.sidecar_mut(npc.entity).unwrap().fractions.hp = remainder;
            let life = zone.binding(npc.entity);
            let effects = zone.sidecar(npc.entity).unwrap().effects.clone();
            let mut trace = Vec::new();
            let result = zone
                .lethal_handler_rolled_v6(
                    zone.next_tick,
                    owner,
                    npc,
                    &skill,
                    1,
                    handler,
                    &mut scripted(
                        &[(R::LethalFull, 1000, 999), (R::LethalHalf, 1000, 0)],
                        &mut trace,
                    ),
                )
                .unwrap();
            assert_eq!(trace.len(), 2);
            assert_eq!(
                (result.target_hp_after, zone.sidecar(npc.entity).unwrap().fractions.hp),
                expected
            );
            assert_eq!(zone.binding(npc.entity), life);
            assert!(!zone.entities[&npc.entity].targeting.dead);
            assert_eq!(zone.sidecar(npc.entity).unwrap().effects, effects);
            assert_eq!(zone.hate, original.hate);
            let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
            let cold = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
            assert_eq!(zone.snapshot(), cold.snapshot());
        }
        let full_skill = original
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(SkillId(1400))
            .unwrap()
            .clone();
        let full_handler = full_skill
            .effects
            .iter()
            .find(|h| h.name == "Lethal")
            .unwrap();
        for full in [false, true] {
            let mut zone = original.clone();
            set_resources(&mut zone, owner, 15, 20);
            zone.sidecar_mut(owner.entity).unwrap().fractions.hp = 500_000;
            zone.sidecar_mut(owner.entity).unwrap().fractions.cp = 250_000;
            let expected = if full {
                vec![(R::LethalFull, 1000, 0)]
            } else {
                vec![(R::LethalFull, 1000, 999), (R::LethalHalf, 1000, 0)]
            };
            let mut trace = Vec::new();
            let result = zone
                .lethal_handler_rolled_v6(
                    zone.next_tick,
                    npc,
                    owner,
                    if full { &full_skill } else { &skill },
                    1,
                    if full { full_handler } else { handler },
                    &mut scripted(&expected, &mut trace),
                )
                .unwrap();
            assert_eq!(trace.len(), expected.len());
            assert_eq!(result.target_hp_after, if full { 1 } else { 15 });
            assert_eq!(
                zone.sidecar(owner.entity).unwrap().fractions.hp,
                if full { 0 } else { 500_000 }
            );
            assert_eq!(zone.sidecar(owner.entity).unwrap().fractions.cp, 0);
            let CombatRole::Player {
                progression: Some(p),
                ..
            } = &zone.entities[&owner.entity].combat.as_ref().unwrap().role
            else {
                unreachable!()
            };
            assert_eq!(p.class_state.cp, 1);
            assert_eq!(result.cp_absorbed, 0);
        }
        let mut zone = original.clone();
        zone.entities
            .get_mut(&npc.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .protected_until = Some(Tick(zone.next_tick.0 + 1));
        let mut trace = Vec::new();
        let result = zone
            .lethal_handler_rolled_v6(
                zone.next_tick,
                owner,
                npc,
                &skill,
                1,
                handler,
                &mut scripted(&[], &mut trace),
            )
            .unwrap();
        assert_eq!(result.outcome, SkillOutcome::Miss);
        assert!(trace.is_empty());
        let mut young = skill.clone();
        young.magic_level = vec![0; usize::try_from(skill.max_level).unwrap()];
        // Raise the actual player sheet above magicLvl0+6; fixture players start at1.
        let xp19 = zone.rules.as_ref().unwrap().xp_to_level(19).unwrap();
        if let CombatRole::Player { xp, .. } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        {
            *xp = xp19;
        }
        zone.relevel(owner.entity).unwrap();
        zone.sync_passives_v5(owner.entity).unwrap();
        zone.rebuild_v5(owner.entity).unwrap();
        zone.lethal_handler_rolled_v6(
            zone.next_tick,
            npc,
            owner,
            &young,
            1,
            handler,
            &mut scripted(&[], &mut trace),
        )
        .unwrap();
        assert!(trace.is_empty());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Shared source fixture compares miss, hit and shield short-circuit traces.
    fn v6_fatal_blow_caller_draw_order_and_perfect_shield_short_circuit() {
        use crate::domain::zone::{
            skill::{RankValue, SkillId},
            EquipSlot,
        };
        use DamageRollSite as R;
        let (mut original, owner, npc) = equipment_fixture();
        original.run_tick(original.draft(Vec::new())).unwrap();
        let skill = original
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(SkillId(16))
            .unwrap()
            .clone();
        let handler = skill
            .effects
            .iter()
            .find(|h| h.name == "FatalBlow")
            .unwrap();
        let mut miss = original.clone();
        let mut trace = Vec::new();
        let result = miss
            .blow_handler_rolled_v6(
                miss.next_tick,
                owner,
                npc,
                &skill,
                1,
                handler,
                0,
                &mut Vec::new(),
                &mut scripted(
                    &[(R::PhysicalEvasion, 100, 99), (R::BlowSuccess, 1000, 999)],
                    &mut trace,
                ),
            )
            .unwrap();
        assert_eq!(result.outcome, SkillOutcome::Miss);
        assert_eq!(trace.len(), 2);
        let mut hit = original.clone();
        let radius = hit
            .effective(owner.entity)
            .unwrap()
            .physical
            .random_damage();
        let rolls = [
            (R::PhysicalEvasion, 100, 99),
            (R::BlowSuccess, 1000, 0),
            (R::Spread, radius * 2 + 1, radius),
            (R::PhysicalVengeance, 100, 99),
        ];
        let mut trace = Vec::new();
        let result = hit
            .blow_handler_rolled_v6(
                hit.next_tick,
                owner,
                npc,
                &skill,
                1,
                handler,
                0,
                &mut Vec::new(),
                &mut scripted(&rolls, &mut trace),
            )
            .unwrap();
        assert!(result.rolled >= 1);
        assert_eq!(result.outcome, SkillOutcome::Hit);
        assert_eq!(trace.len(), 4);
        let mut shield = original;
        let shield_id = crate::domain::zone::ItemInstanceId::new(901).unwrap();
        let player = shield
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        let mut item =
            player.inventory[&crate::domain::zone::ItemInstanceId::new(900).unwrap()].clone();
        item.id = shield_id;
        item.template_id = 628;
        item.count = 1;
        player.inventory.insert(shield_id, item);
        shield.rebuild_v5(owner.entity).unwrap();
        let command = item_intent(
            &shield,
            owner.entity,
            981,
            crate::domain::zone::ItemCommandKind::Equip {
                instance: shield_id,
                preferred_slot: Some(EquipSlot::Lhand),
            },
        );
        let tick = shield
            .run_tick(shield.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::Item(Box::new(command)),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(
            shield.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
                [&EquipSlot::Lhand],
            shield_id
        );
        let before = shield.entities[&npc.entity].pos;
        let pos = shield.entities[&owner.entity].pos;
        shield.entities.get_mut(&npc.entity).unwrap().pos = pos;
        shield.aoi.relocate(npc.entity, before, pos);
        let mut normal = shield.clone();
        let mut trace = Vec::new();
        assert_eq!(
            normal
                .shield_v6(
                    npc,
                    owner,
                    &mut scripted(
                        &[(R::ShieldPerfect, 100, 90), (R::ShieldNormal, 100, 0)],
                        &mut trace
                    )
                )
                .unwrap(),
            crate::domain::zone::phase4::ShieldOutcome::Normal
        );
        assert_eq!(trace.len(), 2);
        let old = normal.entities[&npc.entity].pos;
        let behind = Vec2Fixed::from_tiles(9, 10);
        normal.entities.get_mut(&npc.entity).unwrap().pos = behind;
        normal.aoi.relocate(npc.entity, old, behind);
        let mut trace = Vec::new();
        assert_eq!(
            normal
                .shield_v6(npc, owner, &mut scripted(&[], &mut trace))
                .unwrap(),
            crate::domain::zone::phase4::ShieldOutcome::None
        );
        assert!(trace.is_empty());
        // Source optional critical happens after the perfect-block return and still doubles1.
        let mut critical_handler = handler.clone();
        critical_handler
            .params
            .insert("criticalChance".into(), RankValue::Scalar("100".into()));
        let mut trace = Vec::new();
        let result = shield
            .blow_handler_rolled_v6(
                shield.next_tick,
                npc,
                owner,
                &skill,
                1,
                &critical_handler,
                0,
                &mut Vec::new(),
                &mut scripted(
                    &[
                        (R::PhysicalEvasion, 100, 99),
                        (R::BlowSuccess, 1000, 0),
                        (R::ShieldPerfect, 100, 91),
                        (R::PhysicalCritical, 100_000_000, 0),
                        (R::PhysicalVengeance, 100, 99),
                    ],
                    &mut trace,
                ),
            )
            .unwrap();
        assert_eq!(result.rolled, 2);
        assert_eq!(result.outcome, SkillOutcome::Crit);
        assert_eq!(trace.len(), 5);
    }
    #[test]
    fn v6_blow_xml_order_lethal_can_land_when_fatal_blow_misses() {
        use crate::domain::zone::skill::SkillId;
        let (mut zone, owner, npc) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let skill = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(SkillId(263))
            .unwrap()
            .clone();
        assert_eq!(
            skill
                .effects
                .iter()
                .map(|h| h.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Lethal", "FatalBlow"]
        );
        // Exercise the real XML dispatcher with an actual deterministic RNG position.
        // The learned-skill capability remains deferred until E1's coordinated promotion.
        let (position, expected_end) = (0_u128..10_000)
            .find_map(|position| {
                let mut candidate = zone.clone();
                candidate.rng.set_word_pos(position);
                candidate.roll_below(1000);
                let half = candidate.roll_below(1000);
                candidate.roll_below(100);
                let blow = candidate.roll_below(1000);
                (half < 100 && blow >= 900).then_some((position, candidate.rng.get_word_pos()))
            })
            .unwrap();
        zone.rng.set_word_pos(position);
        set_resources(&mut zone, npc, 1, 0);
        let mut results = Vec::new();
        let mut events = Vec::new();
        zone.impact_target_v5(
            zone.next_tick,
            owner,
            npc,
            &skill,
            1,
            CastId(99),
            &mut events,
            &mut results,
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].outcome, SkillOutcome::Hit);
        assert_eq!(results[1].outcome, SkillOutcome::Miss);
        assert_eq!(zone.entities[&npc.entity].combat.as_ref().unwrap().hp, 0);
        assert_eq!(zone.sidecar(npc.entity).unwrap().fractions.hp, 500_000);
        assert!(!zone.entities[&npc.entity].targeting.dead);
        assert!(events.is_empty());
        assert_eq!(zone.rng.get_word_pos(), expected_end);
    }

    #[test]
    fn v6_retained_creation_clock_survives_equip_shot_consume_and_fresh_grant_in_another_epoch() {
        use crate::domain::zone::item_grant::grant_fingerprint;
        use crate::domain::zone::{
            EquipSlot, ItemCommandKind as K, ItemCreationOrigin, ItemGrantCommand, ZoneId,
        };
        use crate::domain::CharacterId;
        let (mut zone, owner, _, ids) = use_fixture(&[1464]);
        let old = ItemCreationOrigin {
            zone: ZoneId(99),
            epoch: 1,
        };
        let items = zone.phase4.as_mut().unwrap();
        let rules = Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&owner.entity).unwrap();
        let weapon = player.paperdoll[&EquipSlot::Rhand];
        for item in player.inventory.values_mut() {
            item.created_tick = Tick(900);
            item.creation_origin = Some(old);
        }
        let initial = player.inventory.clone();
        let current = ItemCreationOrigin {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
        };
        assert_ne!(old, current);
        assert!(zone.next_tick < Tick(900));
        // A cold retained snapshot keeps the old namespace while using the new zone clock.
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let command = |kind, key, seq| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                seq,
                ZoneCommand::Item(Box::new(item_intent(&zone, owner.entity, key, kind))),
            )
        };
        let grant = ItemGrantCommand {
            entity: owner.entity,
            account: item_intent(
                &zone,
                owner.entity,
                2504,
                K::Unequip {
                    slot: EquipSlot::Rhand,
                },
            )
            .authenticated_account()
            .unwrap(),
            request_id: Uuid::from_u128(2503),
            template_id: 57,
            count: 10,
            fingerprint: grant_fingerprint(CharacterId::from_uuid(owner.entity.as_uuid()), 57, 10),
            rules_hash: rules.hash(),
            paused: None,
            known: None,
            conflict: None,
        };
        let draft = zone.draft(vec![
            command(
                K::Unequip {
                    slot: EquipSlot::Rhand,
                },
                2500,
                1,
            ),
            command(
                K::Equip {
                    instance: weapon,
                    preferred_slot: Some(EquipSlot::Rhand),
                },
                2501,
                2,
            ),
            use_input(&zone, owner, 2502, 3, ids[0], None),
            ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(grant))),
        ]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&weapon].charges, 1);
        assert_eq!(player.inventory[&ids[0]].count, 1);
        for (id, before) in &initial {
            let after = &player.inventory[id];
            assert_eq!(after.created_tick, before.created_tick);
            assert_eq!(after.creation_origin, before.creation_origin);
        }
        let fresh = player
            .inventory
            .values()
            .find(|item| !initial.contains_key(&item.id))
            .unwrap();
        assert_eq!(fresh.template_id, 57);
        assert_eq!(fresh.creation_origin, Some(current));
        assert_eq!(fresh.created_tick, applied.tick);
        assert_eq!(zone.snapshot(), cold.snapshot());
        let restored = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        assert_eq!(restored.state_digest(), zone.state_digest());
    }

    #[test]
    fn v6_item_creation_provenance_changes_digest_and_round_trips() {
        use crate::domain::zone::{EquipSlot, ItemCreationOrigin, ZoneId};
        let (zone, owner, _) = equipment_fixture();
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        let mut baseline = zone.clone();
        let item = baseline
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .get_mut(&weapon)
            .unwrap();
        item.created_tick = Tick(900);
        item.creation_origin = Some(ItemCreationOrigin {
            zone: ZoneId(99),
            epoch: 2,
        });
        let digest = baseline.state_digest();
        assert_ne!(digest, zone.state_digest());
        let restored = ZoneState::from_snapshot_with_items(
            baseline.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        assert_eq!(restored.state_digest(), digest);
        assert_eq!(restored.snapshot(), baseline.snapshot());
        for changed in [
            None,
            Some(ItemCreationOrigin {
                zone: ZoneId(100),
                epoch: 2,
            }),
            Some(ItemCreationOrigin {
                zone: ZoneId(99),
                epoch: 3,
            }),
        ] {
            let mut candidate = baseline.clone();
            candidate
                .phase4
                .as_mut()
                .unwrap()
                .canonical
                .players
                .get_mut(&owner.entity)
                .unwrap()
                .inventory
                .get_mut(&weapon)
                .unwrap()
                .creation_origin = changed;
            assert_ne!(digest, candidate.state_digest());
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_shot_enable_visits_every_toggle_once_in_template_order() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, _, ids) = use_fixture(&[1464, 2511, 3949, 1463]);
        let items = zone.phase4.as_mut().unwrap();
        let rules = Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&owner.entity).unwrap();
        let weapon = player.paperdoll[&EquipSlot::Rhand];
        let revision = player.inventory_revision;
        // Retained toggles may outlive their stack or a previous charge/weapon.
        player.auto_shots.extend([1464, 2511, 3949, 1465]);
        for invalid in [1835, 57, u32::MAX] {
            let mut corrupt = zone.snapshot();
            corrupt
                .phase4
                .as_mut()
                .unwrap()
                .players
                .get_mut(&owner.entity)
                .unwrap()
                .auto_shots
                .insert(invalid);
            assert!(ZoneState::from_snapshot_with_items(corrupt, Some(Arc::clone(&rules))).is_err());
        }
        let snapshot = zone.snapshot();
        let mut cold = ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
        let mut observed = zone.clone();
        observed.enable_items_replay_evidence().unwrap();
        let input = |zone: &ZoneState, key, seq, kind| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                seq,
                ZoneCommand::Item(Box::new(item_intent(zone, owner.entity, key, kind))),
            )
        };
        let draft = zone.draft(vec![input(
            &zone,
            2400,
            1,
            K::ToggleAutoShot {
                template: 1463,
                enabled: true,
            },
        )]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft.clone()).unwrap());
        assert_eq!(applied, observed.run_tick(draft).unwrap());
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_AUTO_ENABLE_RECORDING") {
            use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: snapshot.seed.zone,
                    epoch: snapshot.seed.epoch,
                    records: 1,
                    last_tick: Some(applied.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records: vec![AppliedTickRecord::from_applied(
                    snapshot.seed.zone,
                    &applied,
                )],
                snapshot,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }

        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&weapon].charges, 7);
        assert_eq!(player.inventory_revision, revision + 1);
        assert_eq!(
            player.auto_shots.iter().copied().collect::<Vec<_>>(),
            vec![1463, 1464, 2511, 3949]
        );
        for id in &ids[..3] {
            assert_eq!(player.inventory[id].count, 1);
        }
        assert_eq!(player.inventory[&ids[3]].count, 3);
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.receipts.len(), 1);
        let charges = projection
            .ledger
            .iter()
            .filter(|row| row.operation == ItemOperation::Use)
            .collect::<Vec<_>>();
        assert_eq!(
            charges
                .iter()
                .map(|row| row.operation_ordinal)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            charges
                .iter()
                .map(|row| row.after.as_ref().unwrap().charges)
                .collect::<Vec<_>>(),
            vec![1, 3, 7]
        );
        for row in projection
            .ledger
            .iter()
            .filter(|row| row.operation == ItemOperation::Consume)
        {
            assert_eq!(row.receipt_refs.len(), 1);
            assert_eq!(row.receipt_refs[0].operation, ItemOperation::ToggleAutoShot);
            assert_eq!(row.count_delta, -2);
        }
        let formulas = observed
            .take_items_replay_evidence()
            .into_iter()
            .filter_map(|fact| {
                if let crate::domain::zone::evidence::ItemEvidenceData::Formula(value) = fact.fact {
                    Some(value)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(formulas.len(), 4);
        assert_eq!(
            formulas
                .iter()
                .map(|value| value["actual"]["outcome"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["wrong_grade", "charged", "charged", "charged"]
        );
        // Unequip clears all charge; re-enabling must try every depleted matching stack.
        let draft = zone.draft(vec![input(
            &zone,
            2401,
            2,
            K::Unequip {
                slot: EquipSlot::Rhand,
            },
        )]);
        let tick = zone.run_tick(draft.clone()).unwrap();
        assert!(tick.dispositions.is_empty());
        assert_eq!(tick, cold.run_tick(draft).unwrap());
        let draft = zone.draft(vec![
            input(
                &zone,
                2402,
                3,
                K::Equip {
                    instance: weapon,
                    preferred_slot: Some(EquipSlot::Rhand),
                },
            ),
            input(
                &zone,
                2403,
                4,
                K::ToggleAutoShot {
                    template: 1463,
                    enabled: true,
                },
            ),
        ]);
        let tick = zone.run_tick(draft.clone()).unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(tick, cold.run_tick(draft).unwrap());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&weapon].charges, 0);
        assert_eq!(player.auto_shots.iter().copied().collect::<Vec<_>>(), vec![1463]);
        for id in &ids[..3] {
            assert_eq!(player.inventory[id].count, 1);
        }
        assert_eq!(zone.snapshot(), cold.snapshot());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_shot_manual_toggle_payment_duplicate_and_cold_replay() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, _, ids) = use_fixture(&[1464, 2511, 3949, 1463]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        let revision =
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory_revision;
        let cursor = zone.rng.get_word_pos();
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let mut observed = zone.clone();
        observed.enable_items_replay_evidence().unwrap();
        let draft = zone.draft(vec![
            use_input(&zone, owner, 2300, 1, ids[0], None),
            use_input(&zone, owner, 2300, 2, ids[0], None),
            use_input(&zone, owner, 2301, 3, ids[0], None),
            use_input(&zone, owner, 2302, 4, ids[1], None),
            use_input(&zone, owner, 2303, 5, ids[2], None),
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                6,
                ZoneCommand::Item(Box::new(item_intent(
                    &zone,
                    owner.entity,
                    2304,
                    K::ToggleAutoShot {
                        template: 1463,
                        enabled: true,
                    },
                ))),
            ),
        ]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft.clone()).unwrap());
        assert_eq!(applied, observed.run_tick(draft).unwrap());
        assert_eq!(zone.snapshot(), cold.snapshot());
        assert_eq!(zone.snapshot(), observed.snapshot());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&weapon].charges, 7);
        for id in &ids[..3] {
            assert_eq!(player.inventory[id].count, 1);
        }
        assert_eq!(player.inventory[&ids[3]].count, 3);
        assert!(player.auto_shots.contains(&1463));
        assert_eq!(player.inventory_revision, revision + 1);
        assert_eq!(zone.rng.get_word_pos(), cursor);
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(projection.receipts.len(), 5);
        assert_eq!(
            projection
                .ledger
                .iter()
                .filter(|row| row.operation == ItemOperation::Consume)
                .count(),
            3
        );
        for row in projection
            .ledger
            .iter()
            .filter(|row| row.operation == ItemOperation::Consume)
        {
            assert_eq!(row.count_delta, -2);
            assert_eq!(row.receipt_refs.len(), 1);
        }
        let facts = observed.take_items_replay_evidence();
        let formulas = facts
            .iter()
            .filter_map(|fact| {
                if let crate::domain::zone::evidence::ItemEvidenceData::Formula(value) = &fact.fact
                {
                    Some(value)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(formulas.len(), 5);
        assert_eq!(
            formulas
                .iter()
                .filter(|fact| fact["actual"]["outcome"] == "already_charged")
                .count(),
            1
        );
        assert_eq!(
            formulas
                .iter()
                .filter(|fact| fact["actual"]["outcome"] == "wrong_grade")
                .count(),
            1
        );
        let tick = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                7,
                ZoneCommand::Item(Box::new(item_intent(
                    &zone,
                    owner.entity,
                    2305,
                    K::Unequip {
                        slot: EquipSlot::Rhand,
                    },
                ))),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&weapon].charges, 0);
        for id in &ids[..3] {
            assert_eq!(player.inventory[id].count, 1);
        }
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_shot_reduced_stock_refusal_retains_draw_and_disables_toggle() {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        use crate::infrastructure::eventlog::Recording;
        let (mut zone, owner, _, ids) = use_fixture(&[4814, 1464]);
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .get_mut(&ids[0])
            .unwrap()
            .count = 1;
        let equip = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::Item(Box::new(item_intent(
                    &zone,
                    owner.entity,
                    2400,
                    K::Equip {
                        instance: ids[0],
                        preferred_slot: Some(EquipSlot::Rhand),
                    },
                ))),
            )]))
            .unwrap();
        assert!(equip.dispositions.is_empty(), "{:?}", equip.dispositions);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let snapshot = zone.snapshot();
        let mut cold =
            ZoneState::from_snapshot_with_items(snapshot.clone(), Some(Arc::clone(&rules)))
                .unwrap();
        let mut observed = zone.clone();
        observed.enable_items_replay_evidence().unwrap();
        let before = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
        let mut probe = zone.clone();
        let first = probe.roll_below(100);
        let second = probe.roll_below(100);
        let draft = zone.draft(vec![
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                2,
                ZoneCommand::Item(Box::new(item_intent(
                    &zone,
                    owner.entity,
                    2401,
                    K::ToggleAutoShot {
                        template: 1464,
                        enabled: true,
                    },
                ))),
            ),
            use_input(&zone, owner, 2402, 3, ids[1], None),
        ]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied.dispositions.len(), 1, "{:?}", applied.dispositions);
        assert_eq!(applied.dispositions[0].reason, RejectReason::InsufficientItems);
        assert_eq!(applied, cold.run_tick(draft.clone()).unwrap());
        assert_eq!(applied, observed.run_tick(draft).unwrap());
        assert_eq!(zone.rng.get_word_pos(), probe.rng.get_word_pos());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory, before.inventory);
        assert_eq!(player.inventory_revision, before.inventory_revision);
        assert!(!player.auto_shots.contains(&1464));
        let projection = applied
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(projection.ledger.is_empty());
        assert_eq!(projection.receipts.len(), 1);
        assert_eq!(projection.evidence.len(), 2);
        let facts = observed.take_items_replay_evidence();
        let draws = facts
            .iter()
            .filter_map(|fact| {
                if let crate::domain::zone::evidence::ItemEvidenceData::Rng { value, .. } =
                    &fact.fact
                {
                    Some(*value)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(draws, vec![first, second]);
        assert_eq!(zone.snapshot(), cold.snapshot());
        assert_eq!(zone.snapshot(), observed.snapshot());
        let recording = Recording {
            watermark: Watermark {
                zone: snapshot.seed.zone,
                epoch: snapshot.seed.epoch,
                records: 1,
                last_tick: Some(applied.tick),
                reason: WatermarkReason::Shutdown,
            },
            records: vec![AppliedTickRecord::from_applied(
                snapshot.seed.zone,
                &applied,
            )],
            snapshot,
        };
        let bytes = recording.to_bytes().unwrap();
        let decoded = Recording::from_bytes(&bytes).unwrap();
        assert!(decoded.records[0].reproduced_by(&recording.records[0]));
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_SHOTS_EVIDENCE_RECORDING") {
            std::fs::write(path, bytes).unwrap();
        }
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_shot_automatic_cast_recharges_after_guards_and_retains_paid_charge_on_cancel() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind};
        let (mut zone, owner, npc, ids) = use_fixture(&[2511, 3949, 1463]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        let weapon = player.paperdoll[&EquipSlot::Rhand];
        player.auto_shots.extend([2511, 3949, 1463, 1465]);
        let inventory = player.inventory.clone();
        // Install the real learned skill before the replay boundary.
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut zone
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .role
        else {
            panic!("player")
        };
        p.class_state.learned_skills = vec![crate::domain::subclass::LearnedSkill {
            key: "l2.skill.1177".into(),
            level: 1,
        }];
        zone.rebuild_v5(owner.entity).unwrap();
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let input = |target| {
            review_input(
                owner.entity,
                1,
                ZoneCommand::CastSkill {
                    entity: owner.entity,
                    skill_id: crate::domain::zone::skill::SkillId(1177),
                    target,
                },
            )
        };
        let mut refused = zone.clone();
        let cursor = refused.rng.get_word_pos();
        let bad = refused.run_tick(refused.draft(vec![input(None)])).unwrap();
        assert!(!bad.dispositions.is_empty());
        assert_eq!(refused.rng.get_word_pos(), cursor);
        assert_eq!(
            refused.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory,
            inventory
        );
        assert!(!bad
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(_))));
        zone.enable_items_replay_evidence().unwrap();
        let draft = zone.draft(vec![input(Some(npc.entity))]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        let CastState::Casting(cast) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&weapon].charges, 6);
        assert_eq!(player.inventory[&ids[0]].count, 1);
        assert_eq!(player.inventory[&ids[1]].count, 1);
        assert_eq!(player.inventory[&ids[2]].count, 3);
        assert!(!player.auto_shots.contains(&1465));
        assert!(player.auto_shots.contains(&1463));
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.item_casts[&cast.id].shots, 4);
        let projection = applied
            .events
            .iter()
            .find_map(|e| {
                if let ZoneEvent::ItemCheckpoint(p) = e {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(projection.receipts.is_empty());
        let debits = projection
            .ledger
            .iter()
            .filter(|d| d.operation == ItemOperation::Consume)
            .collect::<Vec<_>>();
        assert_eq!(debits.len(), 2);
        for debit in debits {
            assert_eq!(debit.count_delta, -2);
            assert!(debit.receipt.is_none());
            assert!(debit.receipt_refs.is_empty());
            assert_eq!(debit.causes[0].family, "auto_shot_payment");
        }
        assert_eq!(zone.snapshot(), cold.snapshot());
        let stop = review_input(
            owner.entity,
            2,
            ZoneCommand::CancelCast {
                entity: owner.entity,
                cast_id: Some(cast.id),
            },
        );
        let stopped = zone.run_tick(zone.draft(vec![stop])).unwrap();
        assert!(stopped.dispositions.is_empty());
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .charge_bindings
            .is_empty());
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&weapon]
                .charges,
            6
        );
        // Manual enable while charged cannot debit either stack again.
        let command = item_intent(
            &zone,
            owner.entity,
            2900,
            ItemCommandKind::ToggleAutoShot {
                template: 3949,
                enabled: true,
            },
        );
        let input = review_input(owner.entity, 3, ZoneCommand::Item(Box::new(command)));
        let enabled = zone.run_tick(zone.draft(vec![input])).unwrap();
        assert!(enabled.dispositions.is_empty());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(player.inventory[&ids[0]].count, 1);
        assert_eq!(player.inventory[&ids[1]].count, 1);
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_shot_cast_freezes_exclusive_blessed_timing_damage_cancel_and_cold_replay() {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        use crate::domain::zone::phase4::ItemActionBinding;
        use crate::domain::zone::EquipSlot;
        use crate::infrastructure::eventlog::Recording;
        let (mut zone, owner, npc, ids) = use_fixture(&[2511, 3949]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        let mut probe = zone.clone();
        let cursor = (0_u128..10000)
            .find(|position| {
                probe.rng.set_word_pos(*position);
                probe.roll_below(1000) > 900 && probe.roll_below(100) == 0
            })
            .unwrap();
        zone.rng.set_word_pos(cursor);
        let mut plain = zone.clone();
        plain.run_tick(plain.draft(Vec::new())).unwrap();
        let payment = zone
            .run_tick(zone.draft(vec![
                use_input(&zone, owner, 2500, 1, ids[0], None),
                use_input(&zone, owner, 2501, 2, ids[1], None),
            ]))
            .unwrap();
        assert!(payment.dispositions.is_empty(), "{:?}", payment.dispositions);
        zone.enable_items_replay_evidence().unwrap();
        start_cast(&mut zone, owner, npc);
        start_cast(&mut plain, owner, npc);
        let CastState::Casting(cast) = zone.sidecar(owner.entity).unwrap().cast.clone() else {
            panic!("cast")
        };
        let CastState::Casting(unboosted) = plain.sidecar(owner.entity).unwrap().cast.clone()
        else {
            panic!("cast")
        };
        assert!(cast.complete_at <= unboosted.complete_at);
        let skill = zone
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(cast.skill.id)
            .unwrap()
            .clone();
        let expected = skill_math::cast_animation(
            u32::try_from(*skill.hit_time_ms.at(1, skill.max_level).unwrap()).unwrap(),
            zone.effective(owner.entity).unwrap().cast_speed,
            false,
            true,
            true,
        )
        .unwrap();
        assert_eq!(cast.animation, crate::domain::zone::effects::FrozenRatio::new(expected));
        let facts = zone.take_items_replay_evidence();
        let formula = facts
            .iter()
            .find_map(|fact| {
                if let crate::domain::zone::evidence::ItemEvidenceData::Formula(value) = &fact.fact
                {
                    Some(value)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(formula["formula"], "cast_time");
        assert_eq!(formula["inputs"]["blessed"], true);
        assert_eq!(formula["actual"], u64::try_from(expected.trunc().unwrap()).unwrap());
        let items = &zone.phase4.as_ref().unwrap().canonical;
        assert_eq!(items.item_casts[&cast.id].shots, 4);
        assert_eq!(items.players[&owner.entity].inventory[&weapon].charges, 6);
        assert_eq!(items.charge_bindings.len(), 1);
        assert_eq!(items.charge_bindings[0].action, ItemActionBinding::Cast { id: cast.id });
        assert_eq!(zone.available_cast_shots_v6(owner.entity, &skill).unwrap(), (Some(weapon), 2));
        let snapshot = zone.snapshot();
        for mutation in 0..3 {
            let mut bad = snapshot.clone();
            let items = bad.phase4.as_mut().unwrap();
            match mutation {
                0 => items.charge_bindings.clear(),
                1 => items.charge_bindings.push(items.charge_bindings[0].clone()),
                2 => {
                    items
                        .players
                        .get_mut(&owner.entity)
                        .unwrap()
                        .inventory
                        .get_mut(&weapon)
                        .unwrap()
                        .charges = 2;
                },
                _ => unreachable!(),
            }
            assert!(ZoneState::from_snapshot_with_items(bad, Some(Arc::clone(&rules))).is_err());
        }
        let mut canceled = zone.clone();
        let stop_result = canceled
            .run_tick(canceled.draft(vec![review_input(
                owner.entity,
                3,
                ZoneCommand::CancelCast {
                    entity: owner.entity,
                    cast_id: Some(cast.id),
                },
            )]))
            .unwrap();
        assert!(stop_result.dispositions.is_empty());
        let items = &canceled.phase4.as_ref().unwrap().canonical;
        assert!(items.charge_bindings.is_empty());
        assert!(items.item_casts.is_empty());
        assert_eq!(items.players[&owner.entity].inventory[&weapon].charges, 6);
        let mut cold =
            ZoneState::from_snapshot_with_items(snapshot.clone(), Some(Arc::clone(&rules)))
                .unwrap();
        let mut completed = None;
        let mut records = Vec::new();
        while zone.next_tick <= cast.complete_at {
            let draft = zone.draft(Vec::new());
            let applied = zone.run_tick(draft.clone()).unwrap();
            assert_eq!(applied, cold.run_tick(draft).unwrap());
            records.push(AppliedTickRecord::from_applied(snapshot.seed.zone, &applied));
            if applied.tick == cast.complete_at {
                completed = Some(applied);
            }
        }
        let completed = completed.unwrap();
        let unboosted_result = review_complete(&mut plain, owner);
        let rolled = |applied: &AppliedTick| {
            applied
                .events
                .iter()
                .find_map(|event| {
                    if let ZoneEvent::SkillCastResult {
                        caster, targets, ..
                    } = event
                    {
                        (*caster == owner).then(|| {
                            targets
                                .iter()
                                .find(|target| target.target == npc)
                                .unwrap()
                                .rolled
                        })
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let base = rolled(&unboosted_result);
        let charged = rolled(&completed);
        assert!(base > 1);
        assert!(
            charged >= base.checked_mul(2).unwrap()
                && charged <= base.checked_mul(2).unwrap().checked_add(1).unwrap(),
            "base={base} charged={charged}"
        );
        let items = &zone.phase4.as_ref().unwrap().canonical;
        assert_eq!(items.players[&owner.entity].inventory[&weapon].charges, 2);
        assert!(items.charge_bindings.is_empty());
        assert!(items.item_casts.is_empty());
        for id in ids {
            assert_eq!(items.players[&owner.entity].inventory[&id].count, 1);
        }
        let projection = completed
            .events
            .iter()
            .find_map(|event| {
                if let ZoneEvent::ItemCheckpoint(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(projection.receipts.is_empty());
        assert_eq!(projection.ledger.len(), 1);
        assert_eq!(projection.ledger[0].count_delta, 0);
        assert_eq!(projection.ledger[0].causes[0].family, "shot_discharge");
        assert_eq!(zone.snapshot(), cold.snapshot());
        let recording = Recording {
            watermark: Watermark {
                zone: snapshot.seed.zone,
                epoch: snapshot.seed.epoch,
                records: u64::try_from(records.len()).unwrap(),
                last_tick: Some(completed.tick),
                reason: WatermarkReason::Shutdown,
            },
            snapshot,
            records,
        };
        let bytes = recording.to_bytes().unwrap();
        let decoded = Recording::from_bytes(&bytes).unwrap();
        for (actual, expected) in decoded.records.iter().zip(&recording.records) {
            assert!(actual.reproduced_by(expected));
        }
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_SHOT_CAST_RECORDING") {
            std::fs::write(path, bytes).unwrap();
        }
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_shot_cast_physical_magic_and_heal_caller_terms() {
        use crate::domain::zone::skill::{RankValue, SkillId};
        use DamageRollSite as R;
        let (mut original, owner, npc) = equipment_fixture();
        // The existing controlled getter fixture separates P.Atk from skill power and
        // makes sqrt64/sqrt128/sqrt256 distinguish the three magical branches.
        for id in [owner.entity, npc.entity] {
            pvp_stat_fixture(&mut original, id);
        }
        let registry = Arc::clone(&original.phase3.as_ref().unwrap().canonical.rules);
        let mut physical = registry.skills().get(SkillId(3)).unwrap().clone();
        physical.cast_range = RankValue::Scalar(600);
        let mut handler = physical
            .effects
            .iter()
            .find(|handler| handler.name == "PhysicalAttack")
            .unwrap()
            .clone();
        handler
            .params
            .insert("power".into(), RankValue::Scalar("37".into()));
        handler
            .params
            .insert("criticalChance".into(), RankValue::Scalar("0".into()));
        for (shots, expected) in [(0, 115), (1, 193)] {
            let mut zone = original.clone();
            let mut trace = Vec::new();
            let result = zone
                .physical_handler_rolled_v5(
                    zone.next_tick,
                    owner,
                    npc,
                    &physical,
                    1,
                    &handler,
                    shots,
                    &mut Vec::new(),
                    &mut scripted(&[(R::PhysicalEvasion, 100, 99)], &mut trace),
                )
                .unwrap();
            assert_eq!(result.rolled, expected);
            assert_eq!(trace.len(), 1);
        }
        let mut magic = registry.skills().get(SkillId(1177)).unwrap().clone();
        magic.cast_range = RankValue::Scalar(600);
        let mut handler = magic
            .effects
            .iter()
            .find(|handler| handler.name == "MagicalAttackRange")
            .unwrap()
            .clone();
        handler
            .params
            .insert("power".into(), RankValue::Scalar("7".into()));
        for (shots, expected) in [(0, 56), (2, 79), (4, 112), (6, 112)] {
            let mut zone = original.clone();
            let mut trace = Vec::new();
            let result = zone
                .magic_handler_rolled_v5(
                    zone.next_tick,
                    owner,
                    npc,
                    &magic,
                    1,
                    &handler,
                    shots,
                    &mut Vec::new(),
                    &mut scripted(
                        &[
                            (R::MagicCritical, 1000, 999),
                            (R::MagicSuccess, 100, 0),
                            (R::MagicVengeance, 100, 99),
                        ],
                        &mut trace,
                    ),
                )
                .unwrap();
            assert_eq!(result.rolled, expected);
            assert_eq!(trace.len(), 3);
        }
        let mut heal = registry
            .skills()
            .skills()
            .values()
            .find(|skill| {
                skill.is_magic == 1 && skill.effects.iter().any(|handler| handler.name == "Heal")
            })
            .unwrap()
            .clone();
        heal.mp_consume_end[0] = 10;
        let mut handler = heal
            .effects
            .iter()
            .find(|handler| handler.name == "Heal")
            .unwrap()
            .clone();
        handler
            .params
            .insert("power".into(), RankValue::Scalar("0".into()));
        for (shots, expected) in [(0, 11), (2, 21), (4, 40), (6, 40)] {
            let mut zone = original.clone();
            zone.entities
                .get_mut(&owner.entity)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .hp = 1;
            zone.sidecar_mut(owner.entity).unwrap().fractions.hp = 0;
            let before = zone.rng.get_word_pos();
            let result = zone
                .restore_handler_v5(
                    zone.next_tick,
                    owner,
                    owner,
                    &heal,
                    1,
                    &handler,
                    shots,
                    &mut Vec::new(),
                )
                .unwrap();
            assert_eq!(result.heal, expected);
            assert!(zone.rng.get_word_pos() > before);
        }
    }
}
