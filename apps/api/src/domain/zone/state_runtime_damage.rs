//! Live skill damage callers. Their RNG order is distinct from auto-attacks.
#[path = "state_runtime_auto.rs"]
mod auto;
use super::*;

impl ZoneState {
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
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        self.physical_handler_rolled_v5(
            tick,
            caster,
            target,
            skill,
            rank,
            handler,
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
        .checked_mul(Self::attack_traits_v5(&a, &b, "SWORD")?)
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
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, TickError> {
        self.magic_handler_rolled_v5(
            tick,
            caster,
            target,
            skill,
            rank,
            handler,
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
            q(a.m_atk_q).map_err(invariant)?,
            q(b.m_def_q).map_err(invariant)?,
            resolution,
            critical,
            self.sidecar(caster.entity)?.player.is_some() && self.sidecar(target.entity)?.player.is_some(),
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
}
