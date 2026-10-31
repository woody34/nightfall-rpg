//! Live skill damage callers. Their RNG order is distinct from auto-attacks.
#[path = "state_runtime_auto.rs"]
mod auto;
#[path = "state_runtime_blow.rs"]
mod blow;
#[path = "state_runtime_bow.rs"]
mod bow;
use super::*;
pub(in crate::domain::zone) use auto::AttackStyle;
pub(in crate::domain::zone) use bow::BowStart;

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
    /// D26 owner `EffectiveStats` extension (design §11.2 fields 11..15) from the same folds
    /// the combat callers read: sDef/rShld in Q, shield cone, effective reach and weapon angle.
    pub(super) fn phase4_effective_view_v6(
        &self,
        id: EntityId,
    ) -> Result<crate::domain::zone::item_views::Phase4EffectiveView, TickError> {
        let stats = self.effective(id)?;
        let raw = |value: ExactRatio| -> Result<i64, TickError> {
            i64::try_from(
                value
                    .checked_mul(ExactRatio::whole(Q))
                    .and_then(ExactRatio::trunc)
                    .map_err(invariant)?,
            )
            .map_err(invariant)
        };
        let angle = self
            .damage_stat_v5(id, &stats, "shieldDefAngle", 0)?
            .trunc()
            .map_err(invariant)?
            .checked_add(120)
            .ok_or_else(missing)?
            .clamp(0, 360);
        let combat = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or_else(missing)?;
        Ok(crate::domain::zone::item_views::Phase4EffectiveView {
            shield_def_q: raw(self.damage_stat_v5(id, &stats, "sDef", 0)?)?,
            shield_rate_q: raw(self.damage_stat_v5(id, &stats, "rShld", 0)?)?,
            shield_angle_degrees: u32::try_from(angle).map_err(invariant)?,
            physical_reach_millitiles: i64::from(combat.attack_range.raw()),
            physical_angle_degrees: match self.held_weapon_v6(id) {
                Some((_, definition)) => definition.attack_angle,
                None => self.unarmed_angle_v6(id)?,
            },
        })
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
        let v6 = self.phase4.is_some();
        let debuff = skill.abnormal.as_ref().is_some_and(|a| a.is_debuff);
        // V6 (D2, PhysicalAttack.java:83-91): evasion, then calcShldUse unless
        // ignoreShieldDefence, then the STR critical; legacy keeps evasion→critical.
        let mut shield = crate::domain::zone::phase4::ShieldOutcome::None;
        let Some(critical) = (if v6 {
            let evaded = skill_math::physical_skill_checks(
                skill.is_magic == 1,
                debuff,
                evasion,
                0,
                strength,
                |bound| draw(self, DamageRollSite::PhysicalEvasion, target, bound),
            )
            .map_err(invariant)?
            .is_none();
            if evaded {
                None
            } else {
                let ignore = handler
                    .params
                    .get("ignoreShieldDefence")
                    .and_then(|p| p.at(rank, skill.max_level))
                    .is_some_and(|v| v == "true");
                if !ignore {
                    shield = self.shield_v6(caster, target, draw)?;
                }
                skill_math::physical_skill_checks(
                    true,
                    debuff,
                    evasion,
                    chance,
                    strength,
                    |bound| draw(self, DamageRollSite::PhysicalCritical, target, bound),
                )
                .map_err(invariant)?
            }
        } else {
            skill_math::physical_skill_checks(
                skill.is_magic == 1,
                debuff,
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
        }) else {
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
        // Formulas.java:746-809. Legacy: no shots/shield/facing equipment. V6 (D2): a normal
        // block adds sDef to P.Def (ShieldBlocks=False, character.properties:136), PvP skill
        // defence multiplies it, the actual position multiplies the base, and a perfect
        // block returns 1 before every later term and the spread draw.
        // Penalty uses acting-player level and precedes the max(1) and caller critical.
        // physicalSkillPower has no supported provider in the capability manifest.
        let mut defence = q(b.physical.p_def().raw()).map_err(invariant)?;
        let mut proximity = ExactRatio::whole(1);
        if v6 {
            if shield == crate::domain::zone::phase4::ShieldOutcome::Normal {
                defence = defence
                    .checked_add(
                        ExactRatio::new(
                            self.damage_stat_v5(target.entity, &b, "sDef", 0)?
                                .trunc()
                                .map_err(invariant)?,
                            1,
                        )
                        .map_err(invariant)?,
                    )
                    .map_err(invariant)?;
            }
            defence = defence
                .checked_mul(self.item_pvp_damage_bonus(
                    target.entity,
                    caster.entity,
                    &b,
                    "pvpPhysSkillsDef",
                )?)
                .map_err(invariant)?;
            proximity = self
                .proximity_v6(caster.entity, target.entity)?
                .blow_damage()
                .map_err(invariant)?;
        }
        let perfect = shield == crate::domain::zone::phase4::ShieldOutcome::Perfect;
        let damage = if perfect {
            ExactRatio::whole(1)
        } else {
            self.physical_skill_terms_v6(
                caster, target, &a, &b, power, defence, proximity, pvp_bonus, critical, shots, draw,
            )?
        };
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
        self.physical_skill_finish_v6(
            tick, caster, target, skill, rank, power, &a, &b, critical, damage, shots, result,
            events, draw,
        )
    }

    #[allow(clippy::too_many_arguments)] // Formula inputs captured by the caller in source order.
    fn physical_skill_terms_v6(
        &mut self,
        caster: TargetBinding,
        target: TargetBinding,
        a: &EffectiveStats,
        b: &EffectiveStats,
        power: ExactRatio,
        defence: ExactRatio,
        proximity: ExactRatio,
        pvp_bonus: ExactRatio,
        critical: bool,
        shots: u8,
        draw: &mut DamageRoll<'_>,
    ) -> Result<ExactRatio, TickError> {
        skill_math::physical_skill_base(
            power,
            q(a.physical.p_atk().raw()).map_err(invariant)?,
            defence,
            proximity,
            shots & 1 != 0,
        )
        .map_err(invariant)?
        .checked_mul(pvp_bonus)
        .map_err(invariant)?
        .checked_mul(Self::attack_traits_v5(a, b, self.item_weapon_kind(caster.entity, "SWORD")?)?)
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
        .map_err(invariant)
    }

    #[allow(clippy::too_many_arguments)] // Resolved caller state applied in source order.
    fn physical_skill_finish_v6(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        power: ExactRatio,
        a: &EffectiveStats,
        b: &EffectiveStats,
        critical: bool,
        damage: ExactRatio,
        shots: u8,
        mut result: SkillTargetResult,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<SkillTargetResult, TickError> {
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
        // V6 (D2, MagicalAttack.java:76-78): calcShldUse after the critical draw. A perfect
        // block returns 1 before the success/failure/spread draws (Formulas.java:815-827).
        let shield = if self.phase4.is_some() {
            self.shield_v6(caster, target, draw)?
        } else {
            crate::domain::zone::phase4::ShieldOutcome::None
        };
        if shield == crate::domain::zone::phase4::ShieldOutcome::Perfect {
            self.magic_perfect_evidence_v6(caster, target, skill, rank, handler, shots, critical);
            return self.magic_apply_v6(
                tick,
                caster,
                target,
                critical,
                MagicResolution::Success,
                true,
                1,
                events,
                draw,
            );
        }
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
        // The second (failure) draw is taken only for a player caster after a failed first.
        let second_success = if !success && self.sidecar(caster.entity)?.player.is_some() {
            Some(
                i64::from(
                    draw(self, DamageRollSite::MagicFailure, target, 100).map_err(invariant)?,
                ) < i64::from(rate),
            )
        } else {
            None
        };
        let resolution = if success {
            MagicResolution::Success
        } else if second_success == Some(true)
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
        // A normal block adds sDef, or shieldDefensePercent of it (MagicalAttackRange).
        let mut m_def = q(b.m_def_q).map_err(invariant)?;
        let mut shield_def = ExactRatio::whole(0);
        if shield == crate::domain::zone::phase4::ShieldOutcome::Normal {
            let s_def = ExactRatio::new(
                self.damage_stat_v5(target.entity, &b, "sDef", 0)?
                    .trunc()
                    .map_err(invariant)?,
                1,
            )
            .map_err(invariant)?;
            let percent = handler
                .params
                .get("shieldDefensePercent")
                .and_then(|p| p.at(rank, skill.max_level))
                .map_or(Ok(ExactRatio::whole(0)), |v| ExactRatio::parse_literal(v))
                .map_err(invariant)?;
            let added = if percent
                .checked_cmp(ExactRatio::whole(0))
                .map_err(invariant)?
                .is_gt()
            {
                s_def
                    .checked_mul(percent)
                    .and_then(|v| v.checked_div(ExactRatio::whole(100)))
                    .map_err(invariant)?
            } else {
                s_def
            };
            m_def = m_def.checked_add(added).map_err(invariant)?;
            shield_def = added;
        }
        let pvp_bonus = self.item_pvp_damage_bonus(
            caster.entity,
            target.entity,
            &a,
            if skill.is_magic == 1 {
                "pvpMagicalDmg"
            } else {
                "pvpPhysSkillsDmg"
            },
        )?;
        // Unelemented starter equipment returns 1 before skill attributePower is read.
        let attribute = skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?;
        let npc = self.npc_penalty_v5(
            caster.entity,
            target.entity,
            crate::domain::zone::skill_npc::NpcPenaltyPath::Magical,
        )?;
        let target_player = self.sidecar(target.entity)?.player.is_some();
        let damage = skill_math::magical_skill_base(
            power,
            q(a.m_atk_q)
                .and_then(|value| {
                    value.checked_mul(ExactRatio::whole(if shots & 4 != 0 {
                        4
                    } else if shots & 2 != 0 {
                        2
                    } else {
                        1
                    }))
                })
                .map_err(invariant)?,
            m_def,
            resolution,
            critical,
            self.sidecar(caster.entity)?.player.is_some() && target_player,
        )
        .map_err(invariant)?
        .checked_mul(spread)
        .map_err(invariant)?
        .checked_mul(pvp_bonus)
        .map_err(invariant)?
        .checked_mul(attribute)
        .map_err(invariant)?
        .checked_mul(npc)
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
        if self.items_evidence_enabled() && self.phase4_player_v6(caster.entity) {
            use crate::domain::zone::state::item_evidence_hooks::{
                ratio_json as r, root_floor_certificate,
            };
            let (offset, coefficient, radicand) = damage.parts();
            let scale = ExactRatio::whole(Q);
            let certificate = root_floor_certificate(offset, coefficient, radicand);
            let scaled = offset
                .checked_mul(scale)
                .and_then(|o| coefficient.checked_mul(scale).map(|c| (o, c)))
                .ok()
                .and_then(|(o, c)| root_floor_certificate(o, c, radicand));
            if let (Some((floor, lower, upper)), Some((floor_q, _, _))) = (certificate, scaled) {
                let branch = match resolution {
                    MagicResolution::Half => "half",
                    MagicResolution::Resist => "resisted",
                    MagicResolution::Success if critical => "critical",
                    MagicResolution::Success => "success",
                };
                let level_difference =
                    i64::from(b.physical.level()).saturating_sub(i64::from(a.physical.level()));
                let (pvp_magic, pvp_skill) = if skill.is_magic == 1 {
                    (pvp_bonus, ExactRatio::whole(1))
                } else {
                    (ExactRatio::whole(1), pvp_bonus)
                };
                self.item_formula_v6(
                    caster.entity,
                    "magic_damage",
                    serde_json::json!({"power": r(power), "m_atk": r(q(a.m_atk_q).map_err(invariant)?),
                        "m_def": r(q(b.m_def_q).map_err(invariant)?), "spirit": shots & 2 != 0,
                        "blessed": shots & 4 != 0, "shield_def": r(shield_def), "perfect": false,
                        "critical": critical, "attacker_player": true, "target_player": target_player,
                        "is_pvp": target_player, "skill_is_magic": skill.is_magic == 1,
                        "pvp_magic_def": 1, "pvp_skill_def": 1, "magic_failures": true,
                        "first_success": success, "second_success": second_success,
                        "level_difference": level_difference, "magic_crit_damage": 1,
                        "spread": r(spread), "pvp_magic_damage": r(pvp_magic),
                        "pvp_skill_damage": r(pvp_skill), "attribute": r(attribute), "npc": r(npc)}),
                    serde_json::json!({"lower": lower, "upper": upper, "damage": floor,
                        "damage_q": floor_q, "branch": branch}),
                    &format!("magic:{}:{}:{}", caster.entity, target.entity, self.rng.get_word_pos()),
                    Some(serde_json::json!({"skill_id": skill.id.0, "target_entity": target.entity.to_string(), "event": "cast"})),
                );
            }
        }
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
        self.magic_apply_v6(
            tick, caster, target, critical, resolution, positive, rolled, events, draw,
        )
    }

    /// D46 `magic_damage` for a perfect block: Formulas.java:815-827 returns 1 before any
    /// success/spread draw, so both success fields are null (D89). Evidence-only.
    #[allow(clippy::too_many_arguments)] // The caller's frozen cast inputs.
    fn magic_perfect_evidence_v6(
        &mut self,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        handler: &crate::domain::zone::skill::SkillHandler,
        shots: u8,
        critical: bool,
    ) {
        use crate::domain::zone::state::item_evidence_hooks::ratio_json as r;
        if !self.items_evidence_enabled() || !self.phase4_player_v6(caster.entity) {
            return;
        }
        let (Ok(a), Ok(b)) = (self.effective(caster.entity), self.effective(target.entity)) else {
            return;
        };
        let (Some(power), Ok(m_atk), Ok(m_def), Ok(target_side)) = (
            handler
                .params
                .get("power")
                .and_then(|p| p.at(rank, skill.max_level))
                .and_then(|p| ExactRatio::parse_literal(p).ok()),
            q(a.m_atk_q),
            q(b.m_def_q),
            self.sidecar(target.entity),
        ) else {
            return;
        };
        let target_player = target_side.player.is_some();
        let level_difference =
            i64::from(b.physical.level()).saturating_sub(i64::from(a.physical.level()));
        self.item_formula_v6(
            caster.entity,
            "magic_damage",
            serde_json::json!({"power": r(power), "m_atk": r(m_atk), "m_def": r(m_def),
                "spirit": shots & 2 != 0, "blessed": shots & 4 != 0, "shield_def": 0,
                "perfect": true, "critical": critical, "attacker_player": true,
                "target_player": target_player, "is_pvp": target_player,
                "skill_is_magic": skill.is_magic == 1, "pvp_magic_def": 1, "pvp_skill_def": 1,
                "magic_failures": true, "first_success": null, "second_success": null,
                "level_difference": level_difference, "magic_crit_damage": 1, "spread": 1,
                "pvp_magic_damage": 1, "pvp_skill_damage": 1, "attribute": 1, "npc": 1}),
            serde_json::json!({"lower": 1, "upper": 1, "damage": 1, "damage_q": Q, "branch": "perfect"}),
            &format!("magic:{}:{}:{}", caster.entity, target.entity, self.rng.get_word_pos()),
            Some(serde_json::json!({"skill_id": skill.id.0, "target_entity": target.entity.to_string(), "event": "cast"})),
        );
    }

    /// Shared magical recipient tail; a perfect block enters with rolled damage 1.
    #[allow(clippy::too_many_arguments)] // Resolved caller state applied in source order.
    fn magic_apply_v6(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        critical: bool,
        resolution: MagicResolution,
        positive: bool,
        rolled: u32,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<SkillTargetResult, TickError> {
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

    /// T22: V6 keeps the pinned Phase 3 flag semantics. In V5 and V6 alike, a hostile attack on
    /// a neutral player flags the attacker for `pvp_flag_ticks` (1200) and the victim's
    /// retaliation on the flagged attacker flags the victim for `pvp_flag_low_ticks` (600); each
    /// deadline is the only expiry tick, and both players receive the same `PvpStateChanged`
    /// facts at the same relative ticks (set, then clear exactly at the deadline).
    #[test]
    #[allow(clippy::too_many_lines)] // One scenario run on both runtimes and compared.
    fn t22_pvp_flag_deadline_and_visible_state_match_on_v5_and_v6() {
        // (role: 0 attacker / 1 retaliating victim, tick, flagged, deadline, observers), with
        // ticks relative to the attacker's first flag.
        type Fact = (u8, u64, bool, Option<u64>, usize);
        let run = |mut zone: ZoneState| -> Vec<Fact> {
            let players = zone
                .entities
                .values()
                .filter(|e| e.kind == EntityKind::Player)
                .map(|e| e.id)
                .collect::<Vec<_>>();
            assert_eq!(players.len(), 2);
            let (attacker, victim) = (players[0], players[1]);
            let flags = &zone.phase3.as_ref().unwrap().canonical.rules.pvp().flags;
            assert_eq!((flags.pvp_flag_ticks, flags.pvp_flag_low_ticks), (1200, 600));
            let mut seen: Vec<Fact> = Vec::new();
            let mut start = None;
            let (mut retaliated, mut victim_stopped) = (false, false);
            for step in 0..1600 {
                let mut inputs = Vec::new();
                if step == 0 {
                    inputs.push(ZoneInput::system(ZoneCommand::SetTarget {
                        entity: attacker,
                        target: Some(victim),
                    }));
                    inputs.push(ZoneInput::system(ZoneCommand::Attack { entity: attacker }));
                }
                if start.is_some() && !retaliated {
                    // The attacker stops; the neutral victim strikes the flagged attacker.
                    inputs.push(ZoneInput::system(ZoneCommand::StopAttack { entity: attacker }));
                    inputs.push(ZoneInput::system(ZoneCommand::SetTarget {
                        entity: victim,
                        target: Some(attacker),
                    }));
                    inputs.push(ZoneInput::system(ZoneCommand::Attack { entity: victim }));
                    retaliated = true;
                } else if !victim_stopped && seen.iter().any(|fact| fact.0 == 1) {
                    inputs.push(ZoneInput::system(ZoneCommand::StopAttack { entity: victim }));
                    victim_stopped = true;
                }
                let applied = zone.run_tick(zone.draft(inputs)).unwrap();
                for event in &applied.events {
                    if let ZoneEvent::PvpStateChanged {
                        entity,
                        pvp,
                        baseline: None,
                        ..
                    } = event
                    {
                        let role = u8::from(*entity == victim);
                        let base = *start.get_or_insert(applied.tick.0);
                        // Every player observer receives the fact in the same tick.
                        let observers = players
                            .iter()
                            .filter(|observer| {
                                applied.outputs.get(observer).is_some_and(|outputs| {
                                    outputs.contains(&ObserverOutput::Event(event.clone()))
                                })
                            })
                            .count();
                        seen.push((
                            role,
                            applied.tick.0 - base,
                            pvp.flag_until.is_some(),
                            pvp.flag_until.map(|until| until.0 - base),
                            observers,
                        ));
                    }
                }
                if seen
                    .last()
                    .is_some_and(|(role, _, flagged, _, _)| *role == 0 && !flagged)
                {
                    break;
                }
            }
            seen
        };
        let (v5, ..) = fixture();
        let (v6, ..) = equipment_fixture();
        assert!(v5.phase4.is_none() && v6.phase4.is_some());
        let v5 = run(v5);
        let v6 = run(v6);
        // Neutral target: the attacker is flagged +1200 and cleared exactly at that deadline.
        assert_eq!(v5.first().copied(), Some((0, 0, true, Some(1200), 2)), "{v5:?}");
        assert_eq!(v5.last().copied(), Some((0, 1200, false, None, 2)), "{v5:?}");
        // Retaliation on a flagged player (the recording's path): +600, cleared exactly then.
        let victim = v5
            .iter()
            .filter(|fact| fact.0 == 1)
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(victim.len(), 2, "{v5:?}");
        let (set, deadline) = (victim[0].1, victim[0].3.unwrap());
        assert!(victim[0].2 && victim[0].4 == 2);
        assert_eq!(deadline, set + 600);
        assert_eq!(victim[1], (1, deadline, false, None, 2));
        assert_eq!(v5, v6);
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

    /// T51/D157: a V6 step with actual displacement sets the primitive facing; a blocked
    /// step keeps it; an escape (same-life teleport) keeps it; a new life (town respawn)
    /// starts east. V5 zones have no facing.
    #[test]
    #[allow(clippy::too_many_lines)] // One owner through every facing rule.
    fn v6_facing_follows_actual_movement_and_resets_on_a_new_life() {
        use crate::domain::zone::item_angles::FacingVector;
        // This test forces a zero speed; the T55 per-tick restore check does not apply.
        crate::domain::zone::state::skip_v6_restore_identity();
        let (mut zone, owner, npc) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let facing = |zone: &ZoneState| {
            zone.phase4.as_ref().unwrap().canonical.facing[&owner.entity].components()
        };
        assert_eq!(facing(&zone), FacingVector::EAST.components());
        let walk = |zone: &mut ZoneState, dx: i32, dy: i32| {
            let from = zone.entities[&owner.entity].pos;
            let dest = Vec2Fixed::new(
                Fixed::from_raw(from.x.raw() + dx),
                Fixed::from_raw(from.y.raw() + dy),
            );
            zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::MoveTo {
                entity: owner.entity,
                dest,
            })]))
            .unwrap();
            (from, zone.entities[&owner.entity].pos)
        };
        // North-west diagonal, then due south: each step's displacement, reduced.
        let (from, to) = walk(&mut zone, -3000, 3000);
        assert_ne!(from, to);
        assert_eq!(facing(&zone), (-1, 1));
        zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::StopMove {
            entity: owner.entity,
        })]))
        .unwrap();
        let (from, to) = walk(&mut zone, 0, -4000);
        assert_ne!(from, to);
        assert_eq!(facing(&zone), (0, -1));
        // Blocked (no speed): no displacement, facing kept.
        zone.entities.get_mut(&owner.entity).unwrap().speed = Speed::from_milli_tiles_per_tick(0);
        let (from, to) = walk(&mut zone, 5000, 0);
        assert_eq!(from, to);
        assert_eq!(facing(&zone), (0, -1));
        // Same-life teleport (escape to the safe point) keeps the facing.
        let mut escaped = zone.clone();
        escaped.safe_point = Some(Vec2Fixed::from_tiles(40, 40));
        let before = escaped.entities[&owner.entity].pos;
        escaped
            .escape_v6(escaped.next_tick, owner, &mut Vec::new())
            .unwrap();
        assert_ne!(escaped.entities[&owner.entity].pos, before);
        assert_eq!(facing(&escaped), (0, -1));
        // A new life starts east.
        set_resources(&mut zone, owner, 1, 0);
        zone.apply_damage_rolled_v5(
            zone.next_tick,
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
        assert!(zone.entities[&owner.entity].targeting.dead);
        assert_eq!(facing(&zone), (0, -1), "death itself does not turn the body");
        zone.respawn_town_v5(zone.next_tick, owner.entity).unwrap();
        assert_eq!(facing(&zone), FacingVector::EAST.components());
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
            phase5: None,
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
        let boundary = zone.snapshot();
        let mut observed = zone.clone();
        observed.enable_items_replay_evidence().unwrap();
        let applied = zone.run_tick(draft.clone()).unwrap();
        write_one_tick("NIGHTFALL_TEST_REMOVAL_RECORDING", boundary, &applied);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, observed.run_tick(draft.clone()).unwrap());
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        // D141: the crystallize attests its crystal count under the intent's resolution
        // (`crystallize:<request key>`), owned by the crystallizing player.
        let facts = observed.take_items_replay_evidence();
        let crystal_fact = facts
            .iter()
            .find_map(|f| match &f.fact {
                crate::domain::zone::evidence::ItemEvidenceData::Formula(v)
                    if v["formula"] == "crystal_count" =>
                {
                    Some((f.entity, v.clone()))
                },
                _ => None,
            })
            .expect("crystal_count fact");
        assert_eq!(crystal_fact.0, owner.entity);
        assert_eq!(
            crystal_fact.1["inputs"],
            serde_json::json!({"base": 705, "bonus": 45, "enchant": 4, "weapon": true})
        );
        assert_eq!(crystal_fact.1["actual"], 930);
        assert_eq!(crystal_fact.1["resolution_id"], format!("crystallize:{}", Uuid::from_u128(22)));
        let delta = facts.iter().find_map(|f| match &f.fact {
            crate::domain::zone::evidence::ItemEvidenceData::Delta(v)
                if v["resolution_id"] == crystal_fact.1["resolution_id"] =>
            {
                Some(v.clone())
            },
            _ => None,
        });
        assert!(delta.is_some(), "the crystallize rows share the resolution");
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
    /// Write a one-tick recording (`boundary` = the pre-tick snapshot) when `variable` is set.
    fn write_one_tick(
        variable: &str,
        boundary: crate::domain::zone::ZoneSnapshot,
        applied: &AppliedTick,
    ) {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        if let Ok(path) = std::env::var(variable) {
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: boundary.seed.zone,
                    epoch: boundary.seed.epoch,
                    records: 1,
                    last_tick: Some(applied.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records: vec![AppliedTickRecord::from_applied(boundary.seed.zone, applied)],
                snapshot: boundary,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }
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
            // Evidence never changes the applied tick (compared with `cold` below).
            zone.enable_items_replay_evidence().unwrap();
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
            let player = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
            assert_eq!(player.inventory[&scroll].count, 2);
            assert_eq!(player.inventory_revision, original.inventory_revision + 1);
            assert_eq!(attempt.after.as_ref(), player.inventory.get(&target));
            if matches!(expected, O::SafeFailed | O::SuccessNoIncrement) {
                assert_eq!(player.inventory[&target], original.inventory[&target]);
            }
            // D163: the enchant_outcome fact names the outcome as the receipt does.
            let scrolls_left = player.inventory[&scroll].count;
            let facts = zone.take_items_replay_evidence();
            let fact = facts
                .iter()
                .find_map(|f| match &f.fact {
                    crate::domain::zone::evidence::ItemEvidenceData::Formula(v)
                        if v["formula"] == "enchant_outcome" =>
                    {
                        Some(v.clone())
                    },
                    _ => None,
                })
                .expect("enchant_outcome fact");
            let name = match expected {
                O::Success => "success",
                O::SuccessNoIncrement => "success_no_increment",
                O::SafeFailed => "safe_failure",
                O::BlessedReset => "blessed_failure",
                O::Destroyed => "destroyed",
            };
            assert_eq!(fact["actual"]["outcome"], name, "scroll {template}");
            let level = if expected == O::Destroyed {
                attempt.old_enchant
            } else {
                attempt.new_enchant
            };
            assert_eq!(fact["actual"]["enchant"], level);
            if expected == O::SuccessNoIncrement {
                assert_eq!((attempt.old_enchant, attempt.new_enchant, level), (20, 20, 20));
                assert_eq!(scrolls_left, 2);
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
        // 21040's source cost is item 14701 (itemConsumeId). D97: for_npc no longer defers
        // either template, so without 14701 the use refuses as InsufficientItems before any
        // draw, never substituting the selected item.
        let (mut missing, owner, _, ids) = use_fixture(&[21040]);
        let cursor = missing.rng.get_word_pos();
        let rejected = missing
            .run_tick(missing.draft(vec![use_input(&missing, owner, 960, 1, ids[0], None)]))
            .unwrap();
        assert_eq!(rejected.dispositions[0].reason, RejectReason::InsufficientItems);
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
    /// B4 fixture: a level-52 Bishop at the trainer holding one 8618 stack per `books` count.
    fn bishop_with_books(
        books: &[u64],
    ) -> (
        ZoneState,
        TargetBinding,
        Vec<crate::domain::zone::ItemInstanceId>,
        crate::domain::AccountId,
    ) {
        let (mut zone, owner, _, ids) = use_fixture(&vec![8618; books.len()]);
        let xp_at_52 = zone.rules.as_ref().unwrap().xp_to_level(52).unwrap();
        let base_stats = zone
            .classes
            .as_ref()
            .unwrap()
            .get(ClassId(10))
            .unwrap()
            .base_stats;
        let trainer = Vec2Fixed::from_tiles(126, 128);
        let from = zone.entities[&owner.entity].pos;
        zone.aoi.relocate(owner.entity, from, trainer);
        let entity = zone.entities.get_mut(&owner.entity).unwrap();
        entity.pos = trainer;
        entity.dest = None;
        let CombatRole::Player {
            xp,
            progression: Some(player),
            ..
        } = &mut entity.combat.as_mut().unwrap().role
        else {
            unreachable!()
        };
        *xp = xp_at_52;
        player.class_state.current_class_id = ClassId(16);
        // A fixture-seeded Bishop with both required historical transfer receipts.
        for (target_class, level) in [(15, 20), (16, 40)] {
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
                        position_millitiles: [126_000, 128_000],
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
        let inventory = &mut zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory;
        for (id, count) in ids.iter().zip(books) {
            inventory.get_mut(id).unwrap().count = *count;
        }
        zone.relevel(owner.entity).unwrap();
        zone.sync_passives_v5(owner.entity).unwrap();
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&owner.entity].combat.as_ref().unwrap().role
        else {
            unreachable!()
        };
        let account = player.identity.account_id;
        (zone, owner, ids, account)
    }
    fn book_learn(
        owner: TargetBinding,
        account: crate::domain::AccountId,
        key: Uuid,
        seq: u32,
    ) -> ZoneInput {
        ZoneInput::session(
            owner.entity,
            SessionGeneration(1),
            seq,
            ZoneCommand::LearnSkill {
                entity: owner.entity,
                account,
                request_key: key,
                skill_id: crate::domain::zone::skill::SkillId(1405),
                skill_level: 1,
            },
        )
    }
    fn learned_rank(zone: &ZoneState, owner: TargetBinding) -> u32 {
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&owner.entity].combat.as_ref().unwrap().role
        else {
            unreachable!()
        };
        player
            .class_state
            .learned_skills
            .iter()
            .find(|skill| skill.key == "l2.skill.1405")
            .map_or(0, |skill| skill.level)
    }
    /// E3.4/D47 (B4): a level-52 Bishop learns Divine Inspiration rank 1 by consuming its
    /// book through the ledger (Consume aggregate caused by the learn receipt); without the
    /// book the learn refuses with no effect, and V5 (no inventory) still refuses.
    #[test]
    #[allow(clippy::too_many_lines)] // Fixture-seeded Bishop, refusal, success and V5 parity.
    fn v6_skill_book_learning_consumes_the_book_through_the_ledger() {
        use crate::domain::zone::phase4_projection::{ItemEvidenceValue, ItemOperation};
        use crate::domain::zone::skill::{SkillId, SkillRef};
        // This test removes the book behind the ledger in a refusal branch; the T55 per-tick restore check does not apply.
        crate::domain::zone::state::skip_v6_restore_identity();
        let (mut zone, owner, ids, account) = bishop_with_books(&[1]);
        let book = ids[0];
        let key = Uuid::from_u128(0x0197_0000_0000_7000_8000_0000_0000_0042);
        let learn = |seq| book_learn(owner, account, key, seq);
        let rank = |zone: &ZoneState| learned_rank(zone, owner);
        // Without the book: the read-only query and the command refuse, with no effect.
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
            .remove(&book);
        let generation = SessionGeneration(1);
        assert_eq!(
            missing.validate_learn_skill(
                owner.entity,
                account,
                generation,
                SkillRef {
                    id: SkillId(1405),
                    rank: 1
                }
            ),
            Err(RejectReason::SkillItemRequired)
        );
        let before = missing.snapshot();
        let refused = missing.run_tick(missing.draft(vec![learn(1)])).unwrap();
        assert_eq!(refused.dispositions[0].reason, RejectReason::SkillItemRequired);
        assert!(!refused
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::SkillLearned { .. } | ZoneEvent::ItemCheckpoint(_))));
        assert_eq!(rank(&missing), 0);
        assert_eq!(
            missing.snapshot().phase4,
            before.phase4,
            "a refused learn leaves the inventory untouched"
        );
        // V5 (no inventory) keeps refusing a book-gated row.
        let mut legacy = zone.clone();
        legacy.phase4 = None;
        assert_eq!(
            legacy.validate_learn_skill(
                owner.entity,
                account,
                generation,
                SkillRef {
                    id: SkillId(1405),
                    rank: 1
                }
            ),
            Err(RejectReason::SkillItemRequired)
        );
        // With the book: one tick learns the rank and consumes the book atomically.
        let revision =
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory_revision;
        let snapshot = zone.snapshot();
        let mut cold = ZoneState::from_snapshot_with_items(
            snapshot.clone(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let draft = zone.draft(vec![learn(1)]);
        let learned = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(learned, cold.run_tick(draft).unwrap());
        assert!(learned.dispositions.is_empty(), "{:?}", learned.dispositions);
        assert_eq!(rank(&zone), 1);
        let items = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(!items.inventory.contains_key(&book));
        assert!(items
            .inventory
            .values()
            .all(|item| item.template_id != 8618));
        assert_eq!(items.inventory_revision, revision.checked_add(1).unwrap());
        let learned_at = learned
            .events
            .iter()
            .position(|e| matches!(e, ZoneEvent::SkillLearned { .. }))
            .unwrap();
        let (at, projection) = learned
            .events
            .iter()
            .enumerate()
            .find_map(|(at, e)| match e {
                ZoneEvent::ItemCheckpoint(p) => Some((at, p)),
                _ => None,
            })
            .unwrap();
        assert!(at > learned_at);
        assert!(projection.receipts.is_empty());
        assert_eq!(projection.ledger.len(), 1);
        let row = &projection.ledger[0];
        assert_eq!(row.operation, ItemOperation::Consume);
        assert_eq!(row.instance, book);
        assert_eq!(row.count_delta, -1);
        assert!(row.after.is_none());
        assert_eq!(row.causes.len(), 1);
        assert_eq!(row.causes[0].family, "skill_learn_payment");
        assert_eq!(
            row.causes[0].fields.get("receipt"),
            Some(&ItemEvidenceValue::Text(key.to_string()))
        );
        assert_eq!(row.causes[0].contributing_receipt(), Ok(None));
        assert_eq!(projection.checkpoint.inventory_revision, items.inventory_revision);
        assert!(projection
            .checkpoint
            .instances
            .iter()
            .all(|item| item.template_id != 8618));
        // The same rank cannot be bought twice; the command refuses before any payment.
        let again = zone.run_tick(zone.draft(vec![learn(2)])).unwrap();
        assert_eq!(again.dispositions[0].reason, RejectReason::SkillWrongRank);
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_SKILL_BOOK_LEARN_RECORDING") {
            use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
            let records = vec![AppliedTickRecord::from_applied(
                snapshot.seed.zone,
                &learned,
            )];
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: snapshot.seed.zone,
                    epoch: snapshot.seed.epoch,
                    records: 1,
                    last_tick: Some(learned.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records,
                snapshot,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }
    }
    fn book_checkpoint(tick: &AppliedTick) -> &crate::domain::zone::Phase4Projection {
        let checkpoints: Vec<_> = tick
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) => Some(p.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(checkpoints.len(), 1, "one owner checkpoint per tick");
        checkpoints[0]
    }

    /// engine-review-6 P2-4: two book stacks pay the lowest instance ID, one copy only.
    #[test]
    fn v6_skill_book_learning_pays_the_lowest_instance_of_two_copies() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        let (mut zone, owner, ids, account) = bishop_with_books(&[1, 1]);
        assert!(ids[0] < ids[1]);
        let revision =
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory_revision;
        let key = Uuid::from_u128(0x0197_0000_0000_7000_8000_0000_0000_0043);
        let learned = zone
            .run_tick(zone.draft(vec![book_learn(owner, account, key, 1)]))
            .unwrap();
        assert!(learned.dispositions.is_empty(), "{:?}", learned.dispositions);
        assert_eq!(learned_rank(&zone, owner), 1);
        let items = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(!items.inventory.contains_key(&ids[0]));
        assert_eq!(items.inventory[&ids[1]].count, 1);
        assert_eq!(items.inventory_revision, revision.checked_add(1).unwrap());
        let projection = book_checkpoint(&learned);
        assert_eq!(projection.ledger.len(), 1);
        assert_eq!(projection.ledger[0].operation, ItemOperation::Consume);
        assert_eq!(projection.ledger[0].instance, ids[0]);
        assert_eq!(projection.ledger[0].count_delta, -1);
    }

    /// engine-review-6 P2-4: an earlier item command on the same owner in the same tick merges
    /// with the learn payment: one checkpoint, one revision step, the book's rows in D94 shape.
    #[test]
    fn v6_skill_book_learning_merges_with_an_earlier_same_tick_item_command() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::ItemCommandKind;
        let (mut zone, owner, ids, account) = bishop_with_books(&[2]);
        let book = ids[0];
        let revision =
            zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory_revision;
        let destroy = item_intent(
            &zone,
            owner.entity,
            986,
            ItemCommandKind::Destroy {
                instance: book,
                count: 1,
            },
        );
        let key = Uuid::from_u128(0x0197_0000_0000_7000_8000_0000_0000_0044);
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let draft = zone.draft(vec![
            review_input(owner.entity, 1, ZoneCommand::Item(Box::new(destroy))),
            book_learn(owner, account, key, 2),
        ]);
        let tick = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(tick, cold.run_tick(draft).unwrap());
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(learned_rank(&zone, owner), 1);
        let items = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(items
            .inventory
            .values()
            .all(|item| item.template_id != 8618));
        assert_eq!(items.inventory_revision, revision.checked_add(1).unwrap());
        let learned_at = tick
            .events
            .iter()
            .position(|e| matches!(e, ZoneEvent::SkillLearned { .. }))
            .unwrap();
        let checkpoint_at = tick
            .events
            .iter()
            .position(|e| matches!(e, ZoneEvent::ItemCheckpoint(_)))
            .unwrap();
        assert!(checkpoint_at > learned_at);
        let projection = book_checkpoint(&tick);
        assert_eq!(projection.receipts.len(), 1, "the destroy receipt survives the merge");
        assert_eq!(projection.checkpoint.inventory_revision, items.inventory_revision);
        let rows: Vec<_> = projection
            .ledger
            .iter()
            .filter(|row| row.instance == book)
            .collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].operation, ItemOperation::Destroy);
        assert_eq!(rows[0].count_delta, -1);
        let consume = rows[1];
        assert_eq!(consume.operation, ItemOperation::Consume);
        assert_eq!(consume.count_delta, -1);
        assert!(consume.after.is_none());
        assert_eq!(consume.causes.len(), 1);
        assert_eq!(consume.causes[0].family, "skill_learn_payment");
        assert_eq!(
            rows.iter().map(|row| row.count_delta).sum::<i64>(),
            -2,
            "both copies leave exactly once"
        );
    }

    /// engine-review-6 P2-4: pinned precedence (`RequestAcquireSkill`: SP before items). With
    /// neither the SP nor the book, the refusal is `InsufficientSp` and nothing changes.
    #[test]
    fn v6_skill_book_learning_refuses_sp_before_the_book() {
        use crate::domain::zone::skill::{SkillId, SkillRef};
        // This test edits the class table and the inventory directly; the T55 per-tick restore check does not apply.
        crate::domain::zone::state::skip_v6_restore_identity();
        let (mut zone, owner, ids, account) = bishop_with_books(&[1]);
        let mut encoded = serde_json::to_value(zone.classes.as_deref().unwrap()).unwrap();
        let row = encoded["classes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == 16)
            .unwrap()["skill_tree"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| r["skill_id"] == 1405 && r["skill_level"] == 1)
            .unwrap();
        row["sp_cost"] = serde_json::json!(1000);
        zone.classes = Some(Arc::new(serde_json::from_value(encoded).unwrap()));
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
        player.class_state.sp = 999;
        let query = |zone: &ZoneState| {
            zone.validate_learn_skill(
                owner.entity,
                account,
                SessionGeneration(1),
                SkillRef {
                    id: SkillId(1405),
                    rank: 1,
                },
            )
        };
        // Book present, SP short: SP refuses.
        assert_eq!(query(&zone), Err(RejectReason::InsufficientSp));
        // Neither: SP still refuses first, not the book.
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .remove(&ids[0]);
        assert_eq!(query(&zone), Err(RejectReason::InsufficientSp));
        let before = zone.phase4.as_ref().unwrap().canonical.clone();
        let key = Uuid::from_u128(0x0197_0000_0000_7000_8000_0000_0000_0045);
        let refused = zone
            .run_tick(zone.draft(vec![book_learn(owner, account, key, 1)]))
            .unwrap();
        assert_eq!(refused.dispositions[0].reason, RejectReason::InsufficientSp);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical, before);
        assert_eq!(learned_rank(&zone, owner), 0);
        // SP sufficient, no book: now the book refuses.
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
        player.class_state.sp = 1000;
        assert_eq!(query(&zone), Err(RejectReason::SkillItemRequired));
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
            // Through the cast's completion, so D92's terminal learned_item_cost fact exists.
            let mut records = vec![AppliedTickRecord::from_applied(
                snapshot.seed.zone,
                &started,
            )];
            let mut tail = zone.clone();
            for _ in 0..80 {
                let applied = tail.run_tick(tail.draft(Vec::new())).unwrap();
                records.push(AppliedTickRecord::from_applied(snapshot.seed.zone, &applied));
                if matches!(tail.sidecar(owner.entity).unwrap().cast, CastState::Idle) {
                    break;
                }
            }
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: snapshot.seed.zone,
                    epoch: snapshot.seed.epoch,
                    records: u64::try_from(records.len()).unwrap(),
                    last_tick: records.last().map(|r| r.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records,
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
    /// T31: an enchant whose target and scroll carry D65 creation provenance (granted
    /// instances do). Writes the recording the exporter test strips bindings over.
    #[test]
    fn v6_enchant_with_creation_origin_recording() {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        let (mut live, owner, target, scroll) =
            enchant_fixture(956, crate::domain::zone::EquipSlot::Chest, 0);
        let origin = crate::domain::zone::ItemCreationOrigin {
            zone: live.seed.zone,
            epoch: live.seed.epoch,
        };
        let player = live
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        for id in [target, scroll] {
            player.inventory.get_mut(&id).unwrap().creation_origin = Some(origin);
        }
        live.rebuild_v5(owner.entity).unwrap();
        let snapshot = live.snapshot();
        let draft = live.draft(vec![ZoneInput::session(
            owner.entity,
            SessionGeneration(1),
            1,
            ZoneCommand::Item(Box::new(item_intent(
                &live,
                owner.entity,
                960,
                crate::domain::zone::ItemCommandKind::Enchant {
                    target,
                    scroll,
                    support: None,
                },
            ))),
        )]);
        let applied = live.run_tick(draft).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_ENCHANT_ORIGIN_RECORDING") {
            let records = vec![AppliedTickRecord::from_applied(
                snapshot.seed.zone,
                &applied,
            )];
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: snapshot.seed.zone,
                    epoch: snapshot.seed.epoch,
                    records: 1,
                    last_tick: Some(applied.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records,
                snapshot,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }
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
                ItemEvidenceData::Formula(data) if !load_family(data) => Some(data),
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
        let boundary = zone.snapshot();
        zone.enable_items_replay_evidence().unwrap();
        let cursor = zone.rng.get_word_pos();
        let applied = zone.run_tick(draft.clone()).unwrap();
        write_one_tick("NIGHTFALL_TEST_EXCHANGE_RECORDING", boundary, &applied);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(cursor, zone.rng.get_word_pos());
        let evidence = zone.take_items_replay_evidence();
        let facts = evidence
            .iter()
            .filter_map(|fact| {
                if let crate::domain::zone::evidence::ItemEvidenceData::Formula(value) = &fact.fact
                {
                    (!load_family(value)).then_some(value)
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
    /// Blacksmith fixture: level-46 owner beside the NPC with an unaugmented C-grade necklace
    /// (916, 80 crystals), two accessory stones 12754, D gemstones 2130 and Adena.
    #[allow(clippy::arithmetic_side_effects)]
    /// D94 rows for one instance in the tick's frozen projection.
    fn d94_rows(
        events: &[ZoneEvent],
        instance: crate::domain::zone::ItemInstanceId,
    ) -> Vec<(
        crate::domain::zone::phase4_projection::ItemOperation,
        u32,
        Option<u64>,
        Option<u64>,
    )> {
        let projection = events
            .iter()
            .find_map(|event| match event {
                ZoneEvent::ItemCheckpoint(p) => Some(p),
                _ => None,
            })
            .unwrap();
        projection
            .ledger
            .iter()
            .filter(|row| row.instance == instance)
            .map(|row| {
                (
                    row.operation,
                    row.operation_ordinal,
                    row.before.as_ref().map(|i| i.count),
                    row.after.as_ref().map(|i| i.count),
                )
            })
            .collect()
    }

    #[test]
    fn d94_enchant_then_destroy_freezes_consume_then_terminal_and_replays() {
        use crate::domain::zone::phase4_projection::ItemOperation as Op;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        // api-review-4 poison tick: Enchant{target, scroll} then Destroy{scroll, 2} on 3.
        let (mut zone, owner, target, scroll) = enchant_fixture(956, EquipSlot::Chest, 0);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = session_tick(
            &zone,
            owner.entity,
            vec![
                item_intent(
                    &zone,
                    owner.entity,
                    941,
                    K::Enchant {
                        target,
                        scroll,
                        support: None,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    942,
                    K::Destroy {
                        instance: scroll,
                        count: 2,
                    },
                ),
            ],
        );
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(
            d94_rows(&applied.events, scroll),
            vec![
                (Op::Consume, 0, Some(3), Some(2)),
                (Op::Destroy, 1, Some(2), None)
            ]
        );
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    #[test]
    fn d94_refine_payment_then_destroy_freezes_consume_then_terminal_and_replays() {
        use crate::domain::zone::phase4_projection::ItemOperation as Op;
        use crate::domain::zone::{ItemCommandKind as K, ItemInstanceId};
        // A life stone debited by Refine, then the remaining stack destroyed in the same tick
        // (stones cannot be crystallized; Destroy is the reachable terminal for a payment).
        let (mut zone, owner, npc) = refine_fixture();
        let necklace = ItemInstanceId::new(930).unwrap();
        let stone = ItemInstanceId::new(931).unwrap();
        // Settle the fixture's vitals baseline so live and cold start from one boundary.
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = session_tick(
            &zone,
            owner.entity,
            vec![
                item_intent(
                    &zone,
                    owner.entity,
                    943,
                    K::Equip {
                        instance: necklace,
                        preferred_slot: None,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    944,
                    K::Refine {
                        npc,
                        target: necklace,
                        stone,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    945,
                    K::Destroy {
                        instance: stone,
                        count: 1,
                    },
                ),
            ],
        );
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(
            d94_rows(&applied.events, stone),
            vec![
                (Op::Consume, 0, Some(2), Some(1)),
                (Op::Destroy, 1, Some(1), None)
            ]
        );
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    fn refine_fixture() -> (ZoneState, TargetBinding, TargetBinding) {
        use crate::domain::zone::{ItemInstanceId, ItemLocation};
        let (mut zone, owner, npc) = exchange_fixture_base();
        let rules = zone.rules.clone().unwrap();
        let xp = rules.xp_to_level(46).unwrap();
        {
            let CombatRole::Player { xp: current, .. } = &mut zone
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
            *current = xp;
        }
        zone.relevel(owner.entity).unwrap();
        zone.sync_passives_v5(owner.entity).unwrap();
        let items = zone.phase4.as_mut().unwrap();
        let player = items.canonical.players.get_mut(&owner.entity).unwrap();
        let mut seed = player
            .inventory
            .remove(&ItemInstanceId::new(900).unwrap())
            .unwrap();
        seed.location = ItemLocation::Inventory;
        seed.slot = None;
        seed.augment_id = None;
        seed.enchant = 0;
        for (id, template, count) in [
            (930, 916, 1),
            (931, 12754, 2),
            (932, 2130, 450),
            (933, 57, 500_000),
        ] {
            seed.id = ItemInstanceId::new(id).unwrap();
            seed.template_id = template;
            seed.count = count;
            player.inventory.insert(seed.id, seed.clone());
        }
        zone.rebuild_v5(owner.entity).unwrap();
        zone.validate_runtime_v5().unwrap();
        (zone, owner, npc)
    }

    /// `exchange_fixture` without the SA ingredients: owner moved beside a live blacksmith.
    fn exchange_fixture_base() -> (ZoneState, TargetBinding, TargetBinding) {
        let (mut zone, owner, _) = equipment_fixture();
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
        (zone, owner, npc)
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn session_tick(
        zone: &ZoneState,
        owner: EntityId,
        commands: Vec<crate::domain::zone::ItemCommand>,
    ) -> AppliedTickDraft {
        zone.draft(
            commands
                .into_iter()
                .enumerate()
                .map(|(index, command)| {
                    ZoneInput::session(
                        owner,
                        SessionGeneration(1),
                        u32::try_from(index + 1).unwrap(),
                        ZoneCommand::Item(Box::new(command)),
                    )
                })
                .collect(),
        )
    }

    #[test]
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    fn v6_refine_cancel_refine_pay_exact_inputs_draw_retail_options_and_cold_replay() {
        use crate::domain::zone::evidence::ItemEvidenceData;
        use crate::domain::zone::item_refine::{select, RetailSelectors, RETAIL_RESIDUES};
        use crate::domain::zone::phase4_checkpoint::EquipmentSourceKey;
        use crate::domain::zone::phase4_projection::{ItemOperation, ItemRefineOutcome};
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (mut zone, owner, npc) = refine_fixture();
        let necklace = ItemInstanceId::new(930).unwrap();
        let stone = ItemInstanceId::new(931).unwrap();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let boundary = zone.snapshot();
        let mut cold =
            ZoneState::from_snapshot_with_items(boundary.clone(), Some(Arc::clone(&rules)))
                .unwrap();
        let old = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
        let draft = session_tick(
            &zone,
            owner.entity,
            vec![
                item_intent(
                    &zone,
                    owner.entity,
                    801,
                    K::Equip {
                        instance: necklace,
                        preferred_slot: None,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    802,
                    K::Refine {
                        npc,
                        target: necklace,
                        stone,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    803,
                    K::CancelRefine {
                        npc,
                        target: necklace,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    804,
                    K::Refine {
                        npc,
                        target: necklace,
                        stone,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    805,
                    K::Equip {
                        instance: necklace,
                        preferred_slot: None,
                    },
                ),
            ],
        );
        zone.enable_items_replay_evidence().unwrap();
        let cursor = zone.rng.get_word_pos();
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        // Exactly three retail draws per Refine, none for CancelRefine or Equip.
        assert_eq!(zone.rng.get_word_pos(), cursor + 6);
        let projection = applied
            .events
            .iter()
            .find_map(|event| match event {
                ZoneEvent::ItemCheckpoint(p) => Some(p),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            projection
                .refines
                .iter()
                .map(|r| r.outcome)
                .collect::<Vec<_>>(),
            vec![
                ItemRefineOutcome::Applied,
                ItemRefineOutcome::Removed,
                ItemRefineOutcome::Applied
            ]
        );
        let selectors = RetailSelectors::new(&rules, true, false, 12754).unwrap();
        for audit in projection
            .refines
            .iter()
            .filter(|r| r.outcome == ItemRefineOutcome::Applied)
        {
            let selection = audit.selection.unwrap();
            assert_eq!(
                select(&selectors.first, selection.first_raw % RETAIL_RESIDUES).unwrap(),
                selection.option12
            );
            assert_eq!(
                selectors
                    .category(selection.category_raw % RETAIL_RESIDUES)
                    .unwrap(),
                selection.category
            );
            assert_eq!(
                select(
                    &selectors.category_rows(selection.category),
                    selection.second_raw % RETAIL_RESIDUES
                )
                .unwrap(),
                selection.option34
            );
            assert_eq!(
                audit.after.augment_id,
                Some((selection.option34 << 16) | selection.option12)
            );
            assert_eq!(audit.after.id, necklace);
            assert!(audit.after.slot.is_none());
            assert_eq!(
                audit
                    .payments
                    .iter()
                    .map(|p| (p.template_id, p.count))
                    .collect::<Vec<_>>(),
                vec![(12754, 1), (2130, 200)]
            );
        }
        assert_eq!(projection.refines[0].before.slot, Some(EquipSlot::Neck));
        assert_eq!(projection.refines[1].payments[0].count, 95_000);
        assert!(projection.refines[1].after.augment_id.is_none());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        let after = &player.inventory[&necklace];
        assert_eq!(after.augment_id, projection.refines[2].after.augment_id);
        assert_eq!(after.slot, Some(EquipSlot::Neck));
        assert!(!player.inventory.contains_key(&stone));
        assert_eq!(player.inventory[&ItemInstanceId::new(932).unwrap()].count, 50);
        assert_eq!(player.inventory[&ItemInstanceId::new(933).unwrap()].count, 405_000);
        assert_eq!(player.inventory_revision, old.inventory_revision + 1);
        for half in [0, 1] {
            assert!(player.equipment_sources.values().any(|key| key
                == &EquipmentSourceKey::Augment {
                    instance: necklace,
                    half
                }));
        }
        // One aggregate per paid stack, causes in receipt order (D57/D62); target rows in order.
        for (id, delta, causes) in [(931, -2, 2), (932, -400, 2), (933, -95_000, 1)] {
            let rows: Vec<_> = projection
                .ledger
                .iter()
                .filter(|r| r.instance == ItemInstanceId::new(id).unwrap())
                .collect();
            assert_eq!(rows.len(), 1, "{id}");
            assert_eq!(rows[0].operation, ItemOperation::Consume);
            assert_eq!((rows[0].count_delta, rows[0].causes.len()), (delta, causes));
        }
        let operations: Vec<_> = projection
            .ledger
            .iter()
            .filter(|r| r.instance == necklace)
            .map(|r| r.operation)
            .collect();
        assert_eq!(
            operations,
            vec![
                ItemOperation::Equip,
                ItemOperation::Unequip,
                ItemOperation::Refine,
                ItemOperation::CancelRefine,
                ItemOperation::Refine,
                ItemOperation::Equip
            ]
        );
        let results: Vec<_> = applied
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemResult(r) => Some(r.detail.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 5);
        // D46 evidence: six contiguous draws, two selections + category + packing per Refine,
        // one instance-crystal fee for the cancel.
        let facts = zone.take_items_replay_evidence();
        let draws: Vec<_> = facts
            .iter()
            .filter_map(|f| match &f.fact {
                ItemEvidenceData::Rng {
                    word_before,
                    word_after,
                    bound,
                    callsite,
                    ..
                } => Some((*word_before, *word_after, *bound, callsite.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(draws.len(), 6);
        for pair in draws.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
        assert!(draws
            .iter()
            .all(|d| d.2 == RETAIL_RESIDUES && d.1 == d.0 + 1));
        // Rebuild-family facts (load, D91 clamps, D103 gear_stats of the equips) aside.
        let formulas: Vec<_> = facts
            .iter()
            .filter_map(|f| match &f.fact {
                ItemEvidenceData::Formula(v)
                    if !load_family(v)
                        && !matches!(
                            v["formula"].as_str(),
                            Some("clamp_no_heal" | "gear_stats")
                        ) =>
                {
                    Some(v["formula"].as_str().unwrap().to_owned())
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            formulas,
            [
                "retail_select",
                "category",
                "retail_select",
                "retail_draw_contract",
                "pack_options",
                "cancel_fee",
                "retail_select",
                "category",
                "retail_select",
                "retail_draw_contract",
                "pack_options"
            ]
        );
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
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules)))
                .unwrap()
                .snapshot()
        );
        let sources = zone
            .sidecar(owner.entity)
            .unwrap()
            .modifier_install_order
            .len();
        projection.validate_structure(sources).unwrap();
        for case in 0..7 {
            let mut invalid = projection.as_ref().clone();
            match case {
                0 => invalid.refines.clear(),
                1 => invalid.refines[0].after.augment_id = Some(1 << 16 | 1),
                2 => invalid.refines[0].payments.clear(),
                3 => invalid.refines.swap(0, 1),
                4 => invalid.refines[1].selection = invalid.refines[0].selection,
                5 => invalid.refines[0].after.enchant += 1,
                _ => invalid.refines.push(invalid.refines[0].clone()),
            }
            assert!(invalid.validate_structure(sources).is_err(), "case {case}");
        }
        // Manual CLI/consumer probe uses this exact generated recording, never a synthetic verdict.
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_REFINE_RECORDING") {
            use crate::application::replay_log::{Watermark, WatermarkReason};
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: boundary.seed.zone,
                    epoch: boundary.seed.epoch,
                    records: 1,
                    last_tick: Some(applied.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records: vec![record],
                snapshot: boundary,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_refine_invalid_inputs_and_blockers_do_not_mutate_or_draw() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (zone, owner, npc) = refine_fixture();
        let necklace = ItemInstanceId::new(930).unwrap();
        let stone = ItemInstanceId::new(931).unwrap();
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        let chest = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Chest];
        for case in 0..12 {
            let mut zone = zone.clone();
            let mut kind = K::Refine {
                npc,
                target: necklace,
                stone,
            };
            let expected = match case {
                0 => {
                    // Accessory stone on a weapon: AbstractRefinePacket.java:253-256.
                    kind = K::Refine {
                        npc,
                        target: weapon,
                        stone,
                    };
                    RejectReason::InvalidRefinement
                },
                1 => {
                    kind = K::Refine {
                        npc,
                        target: chest,
                        stone,
                    };
                    RejectReason::InvalidRefinement
                },
                2 => {
                    let player = zone
                        .phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap();
                    player
                        .inventory
                        .get_mut(&ItemInstanceId::new(932).unwrap())
                        .unwrap()
                        .count = 199;
                    RejectReason::InsufficientItems
                },
                3 => {
                    let player = zone
                        .phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap();
                    player.inventory.get_mut(&necklace).unwrap().augment_id = Some((2 << 16) | 1);
                    RejectReason::InvalidRefinement
                },
                4 => {
                    let mut wrong = npc;
                    wrong.incarnation = 1;
                    kind = K::Refine {
                        npc: wrong,
                        target: necklace,
                        stone,
                    };
                    RejectReason::NotPermitted
                },
                5 => {
                    zone.entities.get_mut(&owner.entity).unwrap().pos =
                        Vec2Fixed::from_tiles(127, 129);
                    RejectReason::OutOfRange
                },
                6 => {
                    // D-grade target (AbstractRefinePacket.java:301-303).
                    let player = zone
                        .phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap();
                    player.inventory.get_mut(&necklace).unwrap().template_id = 118;
                    RejectReason::InvalidRefinement
                },
                7 => {
                    kind = K::CancelRefine {
                        npc,
                        target: necklace,
                    };
                    RejectReason::InvalidRefinement
                },
                8 => {
                    let player = zone
                        .phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap();
                    player.inventory.get_mut(&necklace).unwrap().augment_id = Some((2 << 16) | 1);
                    player
                        .inventory
                        .get_mut(&ItemInstanceId::new(933).unwrap())
                        .unwrap()
                        .count = 94_999;
                    kind = K::CancelRefine {
                        npc,
                        target: necklace,
                    };
                    RejectReason::InsufficientItems
                },
                9 => {
                    // A weapon life stone whose options are not all executable stays deferred (D5/D32).
                    let player = zone
                        .phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap();
                    player.inventory.get_mut(&stone).unwrap().template_id = 8723;
                    kind = K::Refine {
                        npc,
                        target: weapon,
                        stone,
                    };
                    RejectReason::ItemCapabilityDeferred
                },
                10 => {
                    kind = K::Refine {
                        npc,
                        target: ItemInstanceId::new(999).unwrap(),
                        stone,
                    };
                    RejectReason::ItemNotOwned
                },
                _ => {
                    // Level below the stone's player level (AbstractRefinePacket.java:261-264).
                    let player = zone
                        .phase4
                        .as_mut()
                        .unwrap()
                        .canonical
                        .players
                        .get_mut(&owner.entity)
                        .unwrap();
                    player.inventory.get_mut(&stone).unwrap().template_id = 12755;
                    RejectReason::InvalidRefinement
                },
            };
            let snapshot = zone.snapshot();
            let command = item_intent(&zone, owner.entity, 810, kind);
            let mut events = Vec::new();
            assert_eq!(
                zone.item_command_v6(zone.next_tick, &command, &mut events, &mut BTreeMap::new())
                    .err(),
                Some(expected),
                "case {case}"
            );
            assert_eq!(zone.snapshot(), snapshot, "case {case}");
            assert!(events.is_empty());
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_cancel_refine_on_equipped_accessory_disarms_weapon_and_removes_providers_once() {
        use crate::domain::zone::phase4_checkpoint::EquipmentSourceKey;
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K, ItemInstanceId};
        let (mut zone, owner, npc) = refine_fixture();
        let necklace = ItemInstanceId::new(930).unwrap();
        let stone = ItemInstanceId::new(931).unwrap();
        let draft = session_tick(
            &zone,
            owner.entity,
            vec![
                item_intent(
                    &zone,
                    owner.entity,
                    820,
                    K::Refine {
                        npc,
                        target: necklace,
                        stone,
                    },
                ),
                item_intent(
                    &zone,
                    owner.entity,
                    821,
                    K::Equip {
                        instance: necklace,
                        preferred_slot: None,
                    },
                ),
            ],
        );
        assert!(zone.run_tick(draft).unwrap().dispositions.is_empty());
        let before = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
        let weapon = before.paperdoll[&EquipSlot::Rhand];
        let hp = zone.entities[&owner.entity].combat.as_ref().unwrap().hp;
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let draft = session_tick(
            &zone,
            owner.entity,
            vec![item_intent(
                &zone,
                owner.entity,
                822,
                K::CancelRefine {
                    npc,
                    target: necklace,
                },
            )],
        );
        let cursor = zone.rng.get_word_pos();
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        assert_eq!(zone.rng.get_word_pos(), cursor);
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        // RequestRefineCancel.java:112-115 disarms the RHAND weapon even for an accessory.
        assert!(!player.paperdoll.contains_key(&EquipSlot::Rhand));
        assert_eq!(player.paperdoll[&EquipSlot::Neck], necklace);
        assert!(player.inventory[&necklace].augment_id.is_none());
        assert!(player.inventory[&weapon].slot.is_none());
        for half in [0, 1] {
            assert!(!player.equipment_sources.values().any(|key| key
                == &EquipmentSourceKey::Augment {
                    instance: necklace,
                    half
                }));
        }
        // Removing the augment re-folds max HP; current HP is clamped to it, never raised.
        let combat = zone.entities[&owner.entity].combat.as_ref().unwrap();
        assert_eq!(combat.hp, hp.min(combat.sheet.max_hp()));
        let projection = applied
            .events
            .iter()
            .find_map(|event| match event {
                ZoneEvent::ItemCheckpoint(p) => Some(p),
                _ => None,
            })
            .unwrap();
        let mut operations: Vec<_> = projection
            .ledger
            .iter()
            .map(|r| (r.instance, r.operation))
            .collect();
        operations.sort();
        let mut expected = vec![
            (ItemInstanceId::new(933).unwrap(), ItemOperation::Consume),
            (weapon, ItemOperation::Unequip),
            (necklace, ItemOperation::CancelRefine),
        ];
        expected.sort();
        assert_eq!(operations, expected);
        assert_eq!(projection.refines[0].payments[0].count, 95_000);
    }

    /// Owner equips `template` (a `use_fixture` item) and stands at (10,10) facing the
    /// fixture NPC at (11,10).
    fn combat_kind_fixture(template: u32) -> (ZoneState, TargetBinding, TargetBinding) {
        let (mut zone, owner, npc, ids) = use_fixture(&[template]);
        let equip = item_intent(
            &zone,
            owner.entity,
            990,
            crate::domain::zone::ItemCommandKind::Equip {
                instance: ids[0],
                preferred_slot: None,
            },
        );
        let tick = zone
            .run_tick(zone.draft(vec![review_input(
                owner.entity,
                1,
                ZoneCommand::Item(Box::new(equip)),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        (zone, owner, npc)
    }

    fn place(zone: &mut ZoneState, entity: EntityId, to: Vec2Fixed) {
        let from = zone.entities[&entity].pos;
        zone.entities.get_mut(&entity).unwrap().pos = to;
        zone.aoi.relocate(entity, from, to);
    }

    fn arm_swing(
        zone: &mut ZoneState,
        owner: TargetBinding,
        target: TargetBinding,
    ) -> crate::domain::zone::Swing {
        let now = zone.next_tick;
        zone.entities
            .get_mut(&owner.entity)
            .unwrap()
            .targeting
            .target = Some(target.entity);
        let swing = crate::domain::zone::Swing {
            target: target.entity,
            target_incarnation: target.incarnation,
            start: now,
            impact: now,
            ready: Tick(now.0.saturating_add(50)),
        };
        zone.entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .swing = Some(swing);
        swing
    }

    /// Deterministic draw source: every hit lands except the listed `AutoHit` ordinals
    /// (1-based), no critical, centred spread, every other site at its last residue.
    #[allow(clippy::arithmetic_side_effects)]
    fn forced<'a>(
        misses: &'a [u32],
        trace: &'a mut Vec<DamageRollSite>,
    ) -> impl FnMut(&mut ZoneState, DamageRollSite, TargetBinding, u32) -> Result<u32, StatError> + 'a
    {
        let mut hits = 0;
        move |zone, site, _, bound| {
            zone.roll_below(bound);
            trace.push(site);
            Ok(match site {
                DamageRollSite::AutoHit => {
                    hits += 1;
                    if misses.contains(&hits) {
                        999
                    } else {
                        0
                    }
                },
                DamageRollSite::AutoCritical => 999,
                DamageRollSite::Spread => bound / 2,
                _ => bound - 1,
            })
        }
    }

    #[test]
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    fn v6_dual_and_player_fist_split_two_halves_with_i3_order_timing_and_stored_second() {
        use crate::domain::zone::state::runtime::AttackStyle;
        use crate::domain::zone::AttackOutcome;
        let (zone, owner, npc) = combat_kind_fixture(253);
        let other = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Player && e.id != owner.entity)
            .unwrap()
            .id;
        assert_eq!(zone.attack_style_v6(owner.entity), AttackStyle::Dual);
        // The unarmed player FIST falls through to the dual handler; NPCs stay simple.
        assert_eq!(zone.attack_style_v6(other), AttackStyle::Dual);
        assert_eq!(zone.attack_style_v6(npc.entity), AttackStyle::Simple);
        // D84: an unarmed human mystic attacks with the class fists weapon 251 (range 40,
        // random damage 5): 40×1000/32 milli-tiles, not the template baseAtkRange 20.
        assert_eq!(
            zone.entities[&other].combat.as_ref().unwrap().attack_range,
            Fixed::from_raw(1250)
        );
        assert_eq!(zone.effective(other).unwrap().physical.random_damage(), 5);
        for misses in [&[][..], &[1][..], &[2][..], &[1, 2][..]] {
            let mut zone = zone.clone();
            let swing = arm_swing(&mut zone, owner, npc);
            let second_at = Tick(swing.impact.0 + 3);
            let hit = |at| crate::domain::zone::phase4::PlannedItemHit {
                target: npc,
                at,
                prepared: None,
            };
            zone.phase4.as_mut().unwrap().canonical.attack_plans.insert(
                owner.entity,
                crate::domain::zone::phase4::ItemAttackPlan {
                    attacker: owner,
                    start: swing.start,
                    ready: swing.ready,
                    weapon: None,
                    shots: 0,
                    arrow: None,
                    hits: vec![hit(swing.impact), hit(second_at)],
                    next_hit: 0,
                },
            );
            let mut probe_trace = Vec::new();
            let (_, full, _, _) = zone
                .clone()
                .auto_resolve_v5(owner, npc, &mut forced(&[], &mut probe_trace))
                .unwrap();
            let hp = zone.entities[&npc.entity].combat.as_ref().unwrap().hp;
            assert!(full / 2 * 2 < hp, "fixture must not kill");
            let mut trace = Vec::new();
            let mut events = Vec::new();
            zone.land_auto_v5(
                swing.impact,
                owner.entity,
                swing,
                &mut events,
                &mut forced(misses, &mut trace),
            )
            .unwrap();
            // I3: hit1, hit2, then each landed hit's critical and spread in order.
            let mut expected = vec![DamageRollSite::AutoHit, DamageRollSite::AutoHit];
            for ordinal in [1, 2] {
                if !misses.contains(&ordinal) {
                    expected.extend([DamageRollSite::AutoCritical, DamageRollSite::Spread]);
                }
            }
            assert_eq!(trace[..expected.len()], expected[..], "{misses:?}");
            let results: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    ZoneEvent::AttackResult {
                        outcome, damage, ..
                    } => Some((*outcome, *damage)),
                    _ => None,
                })
                .collect();
            let first = if misses.contains(&1) {
                (AttackOutcome::Miss, 0)
            } else {
                (AttackOutcome::Hit, full / 2)
            };
            assert_eq!(results, vec![first], "{misses:?}");
            let plan = zone.phase4.as_ref().unwrap().canonical.attack_plans[&owner.entity].clone();
            assert_eq!(plan.next_hit, 1);
            let prepared = plan.hits[1].prepared.clone().unwrap();
            assert_eq!(
                (prepared.outcome, prepared.damage),
                if misses.contains(&2) {
                    (AttackOutcome::Miss, 0)
                } else {
                    (AttackOutcome::Hit, full / 2)
                }
            );
            // The stored second lands without any new hit/critical/spread draw.
            let before = trace.len();
            let mut second_events = Vec::new();
            zone.land_dual_second_v6(
                second_at,
                owner.entity,
                &mut second_events,
                &mut forced(&[], &mut trace),
            )
            .unwrap();
            assert!(trace[before..].iter().all(|site| !matches!(
                site,
                DamageRollSite::AutoHit | DamageRollSite::AutoCritical | DamageRollSite::Spread
            )));
            assert!(second_events
                .iter()
                .any(|e| matches!(e, ZoneEvent::AttackResult { outcome, damage, .. }
                if *outcome == prepared.outcome && *damage == prepared.damage)));
            assert!(!zone
                .phase4
                .as_ref()
                .unwrap()
                .canonical
                .attack_plans
                .contains_key(&owner.entity));
        }
        // A dead recipient, a stunned attacker or a recipient inside the peace square at the
        // deadline discards the stored second result without any further draw (P2-1).
        for case in ["dead", "stunned", "peace"] {
            let mut armed = zone.clone();
            let swing = arm_swing(&mut armed, owner, npc);
            armed
                .phase4
                .as_mut()
                .unwrap()
                .canonical
                .attack_plans
                .insert(
                    owner.entity,
                    crate::domain::zone::phase4::ItemAttackPlan {
                        attacker: owner,
                        start: swing.start,
                        ready: swing.ready,
                        weapon: None,
                        shots: 0,
                        arrow: None,
                        hits: vec![
                            crate::domain::zone::phase4::PlannedItemHit {
                                target: npc,
                                at: swing.impact,
                                prepared: None,
                            },
                            crate::domain::zone::phase4::PlannedItemHit {
                                target: npc,
                                at: Tick(swing.impact.0 + 3),
                                prepared: None,
                            },
                        ],
                        next_hit: 0,
                    },
                );
            let mut trace = Vec::new();
            armed
                .land_auto_v5(
                    swing.impact,
                    owner.entity,
                    swing,
                    &mut Vec::new(),
                    &mut forced(&[], &mut trace),
                )
                .unwrap();
            assert!(armed.phase4.as_ref().unwrap().canonical.attack_plans[&owner.entity].hits[1]
                .prepared
                .is_some());
            match case {
                "dead" => armed.entities.get_mut(&npc.entity).unwrap().targeting.dead = true,
                "stunned" => add_stun(&mut armed, owner),
                _ => {
                    let centre = armed
                        .phase3
                        .as_ref()
                        .unwrap()
                        .canonical
                        .rules
                        .geometry()
                        .center()
                        .unwrap();
                    place(&mut armed, npc.entity, centre);
                },
            }
            let hp = armed.entities[&npc.entity].combat.as_ref().unwrap().hp;
            let drawn = trace.len();
            let mut events = Vec::new();
            armed
                .land_dual_second_v6(
                    Tick(swing.impact.0 + 3),
                    owner.entity,
                    &mut events,
                    &mut forced(&[], &mut trace),
                )
                .unwrap();
            assert!(events.is_empty(), "{case}: {events:?}");
            assert_eq!(trace.len(), drawn, "{case}");
            assert_eq!(armed.entities[&npc.entity].combat.as_ref().unwrap().hp, hp);
            assert!(!armed
                .phase4
                .as_ref()
                .unwrap()
                .canonical
                .attack_plans
                .contains_key(&owner.entity));
        }
        // Live ticks: AttackStarted.impact is the first subhit (timeToHit/2), the second lands
        // at timeToHit, and cold replay from the boundary agrees.
        let mut live = zone.clone();
        let mut cold = ZoneState::from_snapshot_with_items(
            live.snapshot(),
            Some(Arc::clone(&live.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let rules = live.rules.clone().unwrap();
        let speed = live.entities[&owner.entity]
            .combat
            .as_ref()
            .unwrap()
            .sheet
            .attack_speed();
        let timing = crate::domain::zone::attack_timing(rules.constants(), speed).unwrap();
        let first_ms = timing.interval_ms.0 / 2 / 2;
        let first_ticks = u64::try_from((first_ms + 99) / 100).unwrap().max(1);
        let mut started = None;
        let mut landings = Vec::new();
        for step in 0..80 {
            let inputs = if step == 0 {
                vec![
                    review_input(
                        owner.entity,
                        2,
                        ZoneCommand::SetTarget {
                            entity: owner.entity,
                            target: Some(npc.entity),
                        },
                    ),
                    review_input(
                        owner.entity,
                        3,
                        ZoneCommand::Attack {
                            entity: owner.entity,
                        },
                    ),
                ]
            } else {
                Vec::new()
            };
            let draft = live.draft(inputs);
            let applied = live.run_tick(draft.clone()).unwrap();
            assert_eq!(applied, cold.run_tick(draft).unwrap());
            for event in &applied.events {
                match event {
                    ZoneEvent::AttackStarted {
                        attacker,
                        tick,
                        impact,
                        ..
                    } if *attacker == owner.entity && started.is_none() => {
                        started = Some((*tick, *impact));
                    },
                    ZoneEvent::AttackResult { attacker, tick, .. } if *attacker == owner.entity => {
                        landings.push(*tick);
                    },
                    _ => {},
                }
            }
            if landings.len() >= 2 {
                break;
            }
        }
        let (start, impact) = started.unwrap();
        assert_eq!(impact.0 - start.0, first_ticks);
        assert_eq!(landings[..2], [impact, Tick(start.0 + timing.impact_ticks)]);
        assert_eq!(live.snapshot(), cold.snapshot());
    }

    #[test]
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    fn v6_pole_secondaries_follow_ordinal_order_d25_percentages_misses_and_count_bound() {
        use crate::domain::zone::state::runtime::AttackStyle;
        use crate::domain::zone::AttackOutcome;
        let (mut zone, owner, npc) = combat_kind_fixture(15);
        assert_eq!(zone.attack_style_v6(owner.entity), AttackStyle::Pole);
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let template = parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0].clone();
        let spawns: Vec<_> = (0..10)
            .map(|_| {
                ZoneInput::system(ZoneCommand::SpawnNpc {
                    name: "Keltir".into(),
                    pos: Vec2Fixed::from_tiles(30, 30),
                    speed: template.move_speed,
                    combat: Some(Box::new(NpcCombat::from_template(&stats, &template).unwrap())),
                })
            })
            .collect();
        let applied = zone.run_tick(zone.draft(spawns)).unwrap();
        assert!(applied.dispositions.is_empty());
        let mut extra: Vec<_> = zone
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::Npc && e.id != npc.entity && e.combat.is_some())
            .map(|e| e.id)
            .collect();
        extra.sort_by_key(|id| zone.sidecar(*id).unwrap().ordinal);
        assert_eq!(extra.len(), 10);
        let raw = |x, y| Vec2Fixed::new(Fixed::from_raw(x), Fixed::from_raw(y));
        place(&mut zone, owner.entity, raw(10_000, 10_000));
        place(&mut zone, npc.entity, raw(11_000, 10_000));
        // Eight inside range and the 120° cone around the primary heading (east), one behind,
        // one at 90°; spawn order is the entity-ordinal scan order.
        let positions = [
            (10_800, 10_500),
            (11_000, 10_600),
            (11_500, 10_000),
            (11_500, 10_500),
            (11_500, 9_500),
            (10_800, 9_500),
            (11_000, 9_400),
            (12_000, 10_000),
            (9_500, 10_000),
            (10_000, 11_000),
        ];
        let max = zone.entities[&npc.entity]
            .combat
            .as_ref()
            .unwrap()
            .sheet
            .max_hp();
        zone.entities
            .get_mut(&npc.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .hp = max;
        for (id, (x, y)) in extra.iter().zip(positions) {
            place(&mut zone, *id, raw(x, y));
            let max = zone.entities[id].combat.as_ref().unwrap().sheet.max_hp();
            zone.entities
                .get_mut(id)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .hp = max;
            zone.entities.get_mut(id).unwrap().targeting.target = None;
        }
        let maximum = 5; // 1 + skill 3599 atkCountMax add 4 (03500-03599.xml).
        let mut probe = Vec::new();
        let (_, base, _, _) = zone
            .clone()
            .auto_resolve_v5(owner, npc, &mut forced(&[], &mut probe))
            .unwrap();
        // AttackResult reports the HP actually removed from a full-HP recipient.
        let cap = zone.entities[&npc.entity].combat.as_ref().unwrap().hp;
        let expected = |k: u32| -> u32 {
            // trunc(base × 85 × (20/23)^k / 100), exact.
            let numerator = i128::from(base) * 85 * 20_i128.pow(k);
            u32::try_from(numerator / (100 * 23_i128.pow(k)))
                .unwrap()
                .min(cap)
        };
        // A primary miss (draw 1) still scans every secondary (doAttackHitByPole).
        for misses in [&[][..], &[3][..], &[1][..]] {
            let mut zone = zone.clone();
            let swing = arm_swing(&mut zone, owner, npc);
            let mut trace = Vec::new();
            let mut events = Vec::new();
            zone.land_auto_v5(
                swing.impact,
                owner.entity,
                swing,
                &mut events,
                &mut forced(misses, &mut trace),
            )
            .unwrap();
            let results: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    ZoneEvent::AttackResult {
                        target,
                        outcome,
                        damage,
                        ..
                    } => Some((*target, *outcome, *damage)),
                    _ => None,
                })
                .collect();
            // Primary + (count > max) ⇒ max + 1 attempted secondaries.
            assert_eq!(results.len(), 1 + maximum + 1, "{misses:?}");
            if misses.contains(&1) {
                assert_eq!(results[0], (npc.entity, AttackOutcome::Miss, 0));
            } else {
                assert_eq!(results[0], (npc.entity, AttackOutcome::Hit, base.min(cap)));
            }
            for (k, (target, outcome, damage)) in results[1..].iter().enumerate() {
                let k = u32::try_from(k).unwrap();
                assert_eq!(*target, extra[k as usize]);
                if misses.contains(&(k + 2)) {
                    // A miss draws nothing more but still advances the percentage.
                    assert_eq!((*outcome, *damage), (AttackOutcome::Miss, 0));
                } else {
                    assert_eq!((*outcome, *damage), (AttackOutcome::Hit, expected(k)), "k={k}");
                }
            }
            assert!(!results
                .iter()
                .any(|(t, ..)| *t == extra[8] || *t == extra[9]));
        }
        // Player candidates (L2PcInstance.isAutoAttackable): the other player stands first in
        // the cone and in ordinal order; it is skipped while neither flagged nor chaotic, and
        // takes the first 85% secondary when chaotic or flagged, shifting every NPC by one.
        let other = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Player && e.id != owner.entity)
            .unwrap()
            .id;
        assert!(zone.sidecar(other).unwrap().ordinal < zone.sidecar(extra[0]).unwrap().ordinal);
        place(&mut zone, other, raw(10_700, 10_000));
        let other_binding = zone.binding(other).unwrap();
        let mut other_probe = Vec::new();
        let (_, other_base, _, _) = zone
            .clone()
            .auto_resolve_v5(owner, other_binding, &mut forced(&[], &mut other_probe))
            .unwrap();
        for (karma, flagged, included) in [(0, false, false), (1, false, true), (0, true, true)] {
            let mut zone = zone.clone();
            let tick = zone.next_tick;
            let pvp = &mut zone
                .sidecar_mut(other)
                .unwrap()
                .player
                .as_mut()
                .unwrap()
                .pvp;
            pvp.karma = karma;
            pvp.flag_until = flagged.then_some(Tick(tick.0 + 100));
            let swing = arm_swing(&mut zone, owner, npc);
            let mut events = Vec::new();
            zone.land_auto_v5(
                swing.impact,
                owner.entity,
                swing,
                &mut events,
                &mut forced(&[], &mut Vec::new()),
            )
            .unwrap();
            let targets: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    ZoneEvent::AttackResult { target, damage, .. } => Some((*target, *damage)),
                    _ => None,
                })
                .collect();
            assert_eq!(targets.len(), 1 + maximum + 1, "{karma} {flagged}");
            if included {
                let first = u32::try_from(i128::from(other_base) * 85 / 100).unwrap();
                assert_eq!(targets[1].0, other);
                assert_eq!(targets[1].1, first.min(other_base));
                assert_eq!(targets[2].0, extra[0]);
                assert_eq!(targets[2].1, expected(1));
            } else {
                assert!(targets.iter().all(|(t, _)| *t != other));
                assert_eq!(targets[1], (extra[0], expected(0)));
            }
        }
        // A candidate killed by its own secondary (1 HP left) is counted and the scan goes on
        // to the next candidate at the next percentage.
        let mut lethal = zone.clone();
        lethal
            .entities
            .get_mut(&extra[0])
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .hp = 1;
        let swing = arm_swing(&mut lethal, owner, npc);
        let mut events = Vec::new();
        lethal
            .land_auto_v5(
                swing.impact,
                owner.entity,
                swing,
                &mut events,
                &mut forced(&[], &mut Vec::new()),
            )
            .unwrap();
        let results: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::AttackResult { target, damage, .. } => Some((*target, *damage)),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 1 + maximum + 1);
        assert_eq!(results[1], (extra[0], 1));
        assert!(lethal.entities[&extra[0]].targeting.dead);
        assert_eq!(results[2], (extra[1], expected(1)));
        // The fixture must exercise uncapped secondary percentages.
        assert!(expected(1) < cap && expected(1) < expected(0));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_crossbow_refills_arrows_not_bolts_at_attack_time() {
        use crate::domain::zone::item_equipment::PaperdollMutation;
        use crate::domain::zone::EquipSlot;
        // Hand Crossbow 9217 (D), Bone Bolt 9633 (D), Bone Arrow 1341 (D). Crossbows are
        // KAMAEL-conditioned, so the fixture installs the paperdoll directly. D85: the
        // attack-time refill is findArrowForBow (ARROW only), never a bolt.
        for with_arrow in [true, false] {
            let (mut zone, owner, npc, ids) = use_fixture(&[9217, 9633, 1341]);
            let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
            let mut player = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
            if !with_arrow {
                player.inventory.remove(&ids[2]);
            }
            let mutation = PaperdollMutation::equip(
                &player.inventory,
                &player.paperdoll,
                ids[0],
                None,
                &rules,
                player.inventory_revision,
            )
            .unwrap();
            let next = zone
                .phase3
                .as_ref()
                .unwrap()
                .canonical
                .next_install_sequence;
            let (staged, next) = crate::domain::zone::item_sources::apply_paperdoll(
                &player, &mutation, &rules, next,
            )
            .unwrap();
            // Equipping a crossbow auto-equips a bolt (findBoltForCrossBow); take it off so the
            // left hand is empty when the attack starts.
            assert_eq!(staged.paperdoll[&EquipSlot::Lhand], ids[1]);
            let mutation = PaperdollMutation::unequip(
                &staged.inventory,
                &staged.paperdoll,
                EquipSlot::Lhand,
                &rules,
                staged.inventory_revision,
            )
            .unwrap();
            let (staged, next) = crate::domain::zone::item_sources::apply_paperdoll(
                &staged, &mutation, &rules, next,
            )
            .unwrap();
            zone.phase4
                .as_mut()
                .unwrap()
                .canonical
                .players
                .insert(owner.entity, staged);
            zone.phase3
                .as_mut()
                .unwrap()
                .canonical
                .next_install_sequence = next;
            zone.rebuild_v5(owner.entity).unwrap();
            assert!(!zone.bow_attacker_v6(owner.entity));
            assert!(!zone.phase4.as_ref().unwrap().canonical.players[&owner.entity]
                .paperdoll
                .contains_key(&EquipSlot::Lhand));
            let start = zone.entities[&owner.entity].pos;
            place(
                &mut zone,
                npc.entity,
                Vec2Fixed::new(Fixed::from_raw(start.x.raw() + 8_000), start.y),
            );
            let mut started = false;
            for step in 0..10 {
                let inputs = if step == 0 {
                    vec![
                        review_input(
                            owner.entity,
                            2,
                            ZoneCommand::SetTarget {
                                entity: owner.entity,
                                target: Some(npc.entity),
                            },
                        ),
                        review_input(
                            owner.entity,
                            3,
                            ZoneCommand::Attack {
                                entity: owner.entity,
                            },
                        ),
                    ]
                } else {
                    Vec::new()
                };
                let applied = zone.run_tick(zone.draft(inputs)).unwrap();
                if applied.events.iter().any(|e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == owner.entity)) {
                    started = true;
                    break;
                }
            }
            let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
            // Bolts are never taken at attack time.
            assert_eq!(player.inventory[&ids[1]].count, 3);
            if with_arrow {
                assert!(started);
                assert_eq!(player.paperdoll[&EquipSlot::Lhand], ids[2]);
                assert_eq!(player.inventory[&ids[2]].count, 2);
            } else {
                // Only bolts: checkAndEquipArrows fails and the attack intention ends.
                assert!(!started);
                assert!(!player.paperdoll.contains_key(&EquipSlot::Lhand));
            }
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_skill_callers_add_shield_defence_and_perfect_block_returns_one() {
        use crate::domain::zone::EquipSlot;
        use DamageRollSite as R;
        let (mut shield, owner, npc) = equipment_fixture();
        shield.run_tick(shield.draft(Vec::new())).unwrap();
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
            982,
            crate::domain::zone::ItemCommandKind::Equip {
                instance: shield_id,
                preferred_slot: Some(EquipSlot::Lhand),
            },
        );
        let tick = shield
            .run_tick(shield.draft(vec![review_input(
                owner.entity,
                1,
                ZoneCommand::Item(Box::new(command)),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        // Non-degenerate facing (review-3 P2-10): the attacker stands one mirror step in
        // front of the bearer, opposite the "behind" probe point used below.
        let pos = shield.entities[&owner.entity].pos;
        let back = Vec2Fixed::from_tiles(9, 10);
        let front = Vec2Fixed::new(
            Fixed::from_raw(2 * pos.x.raw() - back.x.raw()),
            Fixed::from_raw(2 * pos.y.raw() - back.y.raw()),
        );
        place(&mut shield, npc.entity, front);
        let skills = shield
            .phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .skills()
            .clone();
        let pick = |name: &str| {
            skills
                .values()
                .find(|skill| {
                    skill.runtime_status == crate::domain::class::RuntimeSkillStatus::Executable
                        && skill.effects.iter().any(|h| {
                            h.name == name
                                && !h.params.contains_key("ignoreShieldDefence")
                                && !h.params.contains_key("criticalChance")
                        })
                })
                .unwrap()
                .clone()
        };
        let physical = pick("PhysicalAttack");
        let magical = pick("MagicalAttack");
        // Literal skills from the pinned XML: 1 (ignoreShieldDefence=true) and Wind Strike
        // 1177 (MagicalAttackRange shieldDefensePercent=40).
        let ignoring = skills[&crate::domain::zone::skill::SkillId(1)].clone();
        let percent = skills[&crate::domain::zone::skill::SkillId(1177)].clone();
        // Block modes: Some(true) perfect (ShieldPerfect 99 > 100 − 10); Some(false) normal
        // (90, then ShieldNormal 0 < rShld 20); None unblocked (90, then 99 ≥ 20).
        let run = |zone: &ZoneState,
                   skill: &crate::domain::zone::skill::SkillDef,
                   block: Option<bool>| {
            let mut zone = zone.clone();
            let mut sites = Vec::new();
            let mut draw = |zone: &mut ZoneState,
                            site: R,
                            _: TargetBinding,
                            bound: u32|
             -> Result<u32, StatError> {
                zone.roll_below(bound);
                sites.push(site);
                Ok(match site {
                    R::ShieldPerfect => {
                        if block == Some(true) {
                            99
                        } else {
                            90
                        }
                    },
                    R::ShieldNormal => {
                        if block.is_none() {
                            99
                        } else {
                            0
                        }
                    },
                    R::MagicSuccess => 0,
                    R::PhysicalEvasion => 99,
                    R::MagicCritical => 999,
                    R::Spread => bound / 2,
                    _ => bound - 1,
                })
            };
            let handler = skill
                .effects
                .iter()
                .find(|h| {
                    matches!(
                        h.name.as_str(),
                        "PhysicalAttack" | "MagicalAttack" | "MagicalAttackRange"
                    )
                })
                .unwrap();
            let result = if handler.name == "PhysicalAttack" {
                zone.physical_handler_rolled_v5(
                    zone.next_tick,
                    npc,
                    owner,
                    skill,
                    1,
                    handler,
                    0,
                    &mut Vec::new(),
                    &mut draw,
                )
            } else {
                zone.magic_handler_rolled_v5(
                    zone.next_tick,
                    npc,
                    owner,
                    skill,
                    1,
                    handler,
                    0,
                    &mut Vec::new(),
                    &mut draw,
                )
            }
            .unwrap();
            (result.rolled, sites)
        };
        // PhysicalAttack: evasion → shield → (critical) → formula; perfect returns 1, no spread.
        let (perfect, sites) = run(&shield, &physical, Some(true));
        assert_eq!(perfect, 1);
        assert_eq!(sites[..2], [R::PhysicalEvasion, R::ShieldPerfect]);
        assert!(!sites.contains(&R::Spread));
        let (normal, sites) = run(&shield, &physical, Some(false));
        assert_eq!(sites[..3], [R::PhysicalEvasion, R::ShieldPerfect, R::ShieldNormal]);
        assert!(sites.contains(&R::Spread));
        // MagicalAttack: critical → shield; perfect returns 1 before success and spread draws.
        let (magic_perfect, sites) = run(&shield, &magical, Some(true));
        assert_eq!(magic_perfect, 1);
        assert_eq!(sites[..2], [R::MagicCritical, R::ShieldPerfect]);
        assert!(!sites.contains(&R::MagicSuccess) && !sites.contains(&R::Spread));
        let (magic_normal, sites) = run(&shield, &magical, Some(false));
        assert_eq!(
            sites[..4],
            [
                R::MagicCritical,
                R::ShieldPerfect,
                R::ShieldNormal,
                R::MagicSuccess
            ]
        );
        // Exact source terms: with every other draw equal, damage is floor(K / defence), so a
        // block must be consistent with defence + sDef 128 (Hoplon 628, XML literal) for
        // physical and magical callers, and + 128 × 40 / 100 for Wind Strike.
        let b = shield.effective(owner.entity).unwrap();
        let (p_def, m_def) = (i128::from(b.physical.p_def().raw()), i128::from(b.m_def_q));
        let q = i128::from(Q);
        let consistent = |open: u32, blocked: u32, defence: i128, added: i128| {
            // K·Q ∈ [open·d, (open+1)·d) ∩ [blocked·(d+a), (blocked+1)·(d+a)), all ×Q.
            let (open, blocked) = (i128::from(open), i128::from(blocked));
            let with = defence + added;
            (open * defence).max(blocked * with) < ((open + 1) * defence).min((blocked + 1) * with)
        };
        let (unblocked, sites) = run(&shield, &physical, None);
        assert_eq!(sites[..3], [R::PhysicalEvasion, R::ShieldPerfect, R::ShieldNormal]);
        assert!(normal < unblocked);
        assert!(consistent(unblocked, normal, p_def, 128 * q), "{unblocked} {normal}");
        assert!(!consistent(unblocked, normal, p_def, 0));
        let (magic_unblocked, _) = run(&shield, &magical, None);
        assert!(consistent(magic_unblocked, magic_normal, m_def, 128 * q));
        let (wind_unblocked, _) = run(&shield, &percent, None);
        let (wind_blocked, sites) = run(&shield, &percent, Some(false));
        assert!(sites.contains(&R::ShieldNormal));
        assert!(consistent(wind_unblocked, wind_blocked, m_def, 128 * q * 40 / 100));
        assert!(!consistent(wind_unblocked, wind_blocked, m_def, 128 * q));
        // ignoreShieldDefence skips both shield draws and the defence term.
        let (ignored, sites) = run(&shield, &ignoring, Some(false));
        assert!(!sites.contains(&R::ShieldPerfect) && !sites.contains(&R::ShieldNormal));
        assert_eq!(ignored, run(&shield, &ignoring, None).0);
        // Behind the shield bearer: no eligible block, no shield draws, larger damage.
        let mut behind = shield.clone();
        place(&mut behind, npc.entity, Vec2Fixed::from_tiles(9, 10));
        let (open, sites) = run(&behind, &physical, Some(false));
        assert!(!sites.contains(&R::ShieldPerfect));
        assert!(open > normal, "{open} > {normal}");
        let (magic_open, sites) = run(&behind, &magical, Some(false));
        assert!(!sites.contains(&R::ShieldPerfect));
        assert!(magic_open > magic_normal, "{magic_open} > {magic_normal}");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_d26_spawn_and_owner_stats_carry_phase4_enrichment_through_record_and_wire() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        use crate::interface::grpc::pb;
        let (mut zone, owner, _) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let other = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Player && e.id != owner.entity)
            .unwrap()
            .id;
        let home = zone.entities[&owner.entity].pos;
        place(&mut zone, owner.entity, Vec2Fixed::from_tiles(200, 200));
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        place(&mut zone, owner.entity, home);
        let applied = zone.run_tick(zone.draft(Vec::new())).unwrap();
        let spawn = applied.outputs[&other]
            .iter()
            .find_map(|o| match o {
                ObserverOutput::Event(event @ ZoneEvent::EntitySpawn { entity, .. })
                    if *entity == owner.entity =>
                {
                    Some(event.clone())
                },
                _ => None,
            })
            .unwrap();
        let ZoneEvent::EntitySpawn {
            phase4: Some(items),
            ..
        } = &spawn
        else {
            panic!("V6 spawn must carry its visible equipment")
        };
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(items.visible.len(), player.paperdoll.len());
        assert!(items
            .visible
            .iter()
            .any(|v| v.slot == EquipSlot::Rhand && v.template_id == 71));
        // The unequipped peer's spawn is present with an empty visible list.
        let peer = applied.outputs[&owner.entity]
            .iter()
            .find_map(|o| match o {
                ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, phase4, .. })
                    if *entity == other =>
                {
                    Some(phase4.clone())
                },
                _ => None,
            })
            .unwrap()
            .unwrap();
        assert!(peer.visible.is_empty());
        let wire = crate::interface::zone_mapping::world_event_to_pb(&spawn, 0);
        let Some(pb::world_event::Event::Spawn(message)) = &wire[0].event else {
            panic!("spawn")
        };
        assert_eq!(message.visible_equipment.len(), items.visible.len());
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
        // Owner stats after an inventory revision: weight, penalties, revision and effective
        // extension, mapped to StatsChanged 24..26 and EffectiveStats 11..15.
        let command = item_intent(
            &zone,
            owner.entity,
            991,
            K::Unequip {
                slot: EquipSlot::Chest,
            },
        );
        let applied = zone
            .run_tick(zone.draft(vec![review_input(
                owner.entity,
                1,
                ZoneCommand::Item(Box::new(command)),
            )]))
            .unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let stats = applied.outputs[&owner.entity]
            .iter()
            .find_map(|o| match o {
                ObserverOutput::Event(
                    event @ ZoneEvent::StatsChanged {
                        phase4: Some(_), ..
                    },
                ) => Some(event.clone()),
                _ => None,
            })
            .unwrap();
        let ZoneEvent::StatsChanged {
            phase4: Some(enrichment),
            ..
        } = &stats
        else {
            unreachable!()
        };
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(enrichment.inventory_revision, player.inventory_revision);
        assert_eq!(enrichment.weight.weight, player.weight);
        assert_eq!(enrichment.penalties.overload_level, player.overload_level);
        assert_eq!(
            enrichment.effective.physical_reach_millitiles,
            i64::from(
                zone.entities[&owner.entity]
                    .combat
                    .as_ref()
                    .unwrap()
                    .attack_range
                    .raw()
            )
        );
        assert_eq!(enrichment.effective.physical_angle_degrees, 120);
        let wire = crate::interface::zone_mapping::world_event_to_pb(&stats, 0);
        let Some(pb::world_event::Event::StatsChanged(message)) = &wire[0].event else {
            panic!("stats")
        };
        assert_eq!(message.inventory_revision, player.inventory_revision);
        assert_eq!(message.item_weight.as_ref().unwrap().weight, player.weight);
        let effective = message.effective_stats.as_ref().unwrap();
        assert_eq!(
            effective.physical_reach_millitiles,
            enrichment.effective.physical_reach_millitiles
        );
        assert_eq!(effective.shield_angle_degrees, enrichment.effective.shield_angle_degrees);
        let record = crate::application::replay_log::AppliedTickRecord::from_applied(
            zone.seed.zone,
            &applied,
        );
        assert_eq!(
            crate::application::replay_log::AppliedTickRecord::decode(&record.encode()).unwrap(),
            record
        );
    }

    /// Recorded dual then pole auto attacks with D46 evidence enabled; the facts must be
    /// reproduced by an independent cold replay. `NIGHTFALL_TEST_COMBAT_RECORDING` writes the
    /// exact recording for the CLI exporter and the frozen harness consumer.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn v6_combat_kind_evidence_recording_replays_cold() {
        use crate::application::replay_log::{AppliedTickRecord, Watermark, WatermarkReason};
        use crate::domain::zone::evidence::ItemEvidenceData;
        use crate::domain::zone::ItemCommandKind as K;
        let (mut zone, owner, npc, ids) = use_fixture(&[253, 15]);
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let template = parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0].clone();
        // Two more Keltirs inside the later pole cone; NPC spawn draws precede the boundary.
        let spawns = [(10_800, 10_400), (11_000, 9_600)]
            .into_iter()
            .map(|(x, y)| {
                ZoneInput::system(ZoneCommand::SpawnNpc {
                    name: "Keltir".into(),
                    pos: Vec2Fixed::new(Fixed::from_raw(x), Fixed::from_raw(y)),
                    speed: template.move_speed,
                    combat: Some(Box::new(NpcCombat::from_template(&stats, &template).unwrap())),
                })
            })
            .collect();
        assert!(zone
            .run_tick(zone.draft(spawns))
            .unwrap()
            .dispositions
            .is_empty());
        let boundary = zone.snapshot();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut cold =
            ZoneState::from_snapshot_with_items(boundary.clone(), Some(Arc::clone(&rules)))
                .unwrap();
        zone.enable_items_replay_evidence().unwrap();
        let mut records = Vec::new();
        let mut facts = Vec::new();
        let mut seq = 0_u32;
        let mut next = |zone: &ZoneState, commands: Vec<ZoneCommand>| {
            zone.draft(
                commands
                    .into_iter()
                    .map(|command| {
                        seq += 1;
                        ZoneInput::session(owner.entity, SessionGeneration(1), seq, command)
                    })
                    .collect(),
            )
        };
        let mut phases = vec![
            vec![ZoneCommand::Item(Box::new(item_intent(
                &zone,
                owner.entity,
                1201,
                K::Equip {
                    instance: ids[0],
                    preferred_slot: None,
                },
            )))],
            vec![
                ZoneCommand::SetTarget {
                    entity: owner.entity,
                    target: Some(npc.entity),
                },
                ZoneCommand::Attack {
                    entity: owner.entity,
                },
            ],
        ];
        let mut landings = 0;
        for step in 0..60 {
            let commands = if phases.is_empty() {
                Vec::new()
            } else {
                phases.remove(0)
            };
            let draft = next(&zone, commands);
            if step == 20 {
                phases.push(vec![ZoneCommand::Item(Box::new(item_intent(
                    &zone,
                    owner.entity,
                    1202,
                    K::Equip {
                        instance: ids[1],
                        preferred_slot: None,
                    },
                )))]);
                phases.push(vec![
                    ZoneCommand::SetTarget {
                        entity: owner.entity,
                        target: Some(npc.entity),
                    },
                    ZoneCommand::Attack {
                        entity: owner.entity,
                    },
                ]);
            }
            let applied = zone.run_tick(draft.clone()).unwrap();
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
            assert_eq!(applied, cold.run_tick(draft).unwrap());
            landings += applied
                .events
                .iter()
                .filter(|e| matches!(e, ZoneEvent::AttackResult { attacker, .. } if *attacker == owner.entity))
                .count();
            records.push(AppliedTickRecord::from_applied(zone.seed.zone, &applied));
            let tick_facts = zone.take_items_replay_evidence();
            facts.extend(tick_facts);
        }
        assert!(landings >= 3);
        let names: std::collections::BTreeSet<_> = facts
            .iter()
            .filter_map(|f| match &f.fact {
                ItemEvidenceData::Formula(v) => v["formula"].as_str().map(str::to_owned),
                _ => None,
            })
            .collect();
        for required in [
            "dual_timing",
            "dual_damage",
            "dual_plan",
            "pole_sequence",
            "physical_damage",
            "fist_spread",
            "position",
        ] {
            assert!(names.contains(required), "{required}: {names:?}");
        }
        if let Ok(path) = std::env::var("NIGHTFALL_TEST_COMBAT_RECORDING") {
            let recording = crate::infrastructure::eventlog::Recording {
                watermark: Watermark {
                    zone: boundary.seed.zone,
                    epoch: boundary.seed.epoch,
                    records: u64::try_from(records.len()).unwrap(),
                    last_tick: records.last().map(|r| r.tick),
                    reason: WatermarkReason::Shutdown,
                },
                records,
                snapshot: boundary,
            };
            std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
        }
    }

    /// Rebuild-time load/penalty facts accompany any weight change; operation-focused tests
    /// count only their own formula families.
    fn load_family(value: &serde_json::Value) -> bool {
        matches!(value["formula"].as_str(), Some("max_load" | "overload" | "grade_penalties"))
    }

    #[test]
    fn load_family_resolution_ids_are_unique_within_a_tick() {
        use crate::domain::zone::evidence::ItemEvidenceData;
        use crate::domain::zone::ItemCommandKind as K;
        // Review-3 P2-8: two weight-changing commands with no draw between them.
        let (mut zone, owner, _, ids) = use_fixture(&[13, 15]);
        zone.enable_items_replay_evidence().unwrap();
        let draft = session_tick(
            &zone,
            owner.entity,
            ids.iter()
                .zip(961_u128..)
                .map(|(id, request)| {
                    item_intent(
                        &zone,
                        owner.entity,
                        request,
                        K::Destroy {
                            instance: *id,
                            count: 1,
                        },
                    )
                })
                .collect(),
        );
        let applied = zone.run_tick(draft).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let mut seen = std::collections::BTreeSet::new();
        let mut rebuilds = std::collections::BTreeSet::new();
        for fact in zone.take_items_replay_evidence() {
            if let ItemEvidenceData::Formula(value) = &fact.fact {
                if load_family(value) {
                    let id = value["resolution_id"].as_str().unwrap().to_owned();
                    let formula = value["formula"].as_str().unwrap().to_owned();
                    assert!(seen.insert((formula, id.clone())), "duplicate {id}");
                    rebuilds.insert(id);
                }
            }
        }
        assert_eq!(rebuilds.len(), 2, "{rebuilds:?}");
    }

    /// D4/C10 measurement gate: the V6 snapshot envelope with eight fully loaded owners
    /// (200 storable instances each, a full paperdoll with enchant and augment values, its
    /// providers/sets, high-entropy maximal-width scalars) must stay inside the broker and
    /// decoded budgets (record.rs `MAX_SNAPSHOT_ENCODED` / `MAX_SNAPSHOT_JSON`), decode, and
    /// restore through `from_snapshot_with_items` (review-3 P2-9). Rules are hash-referenced
    /// (D27) and are not part of the envelope.
    #[test]
    #[allow(
        clippy::print_stderr,
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        clippy::too_many_lines
    )]
    fn v6_c10_snapshot_envelope_with_eight_full_inventories_fits_budgets() {
        use crate::application::replay_log::{
            decode_snapshot, encode_snapshot, MAX_SNAPSHOT_ENCODED, MAX_SNAPSHOT_JSON,
        };
        use crate::domain::zone::item_equipment::PaperdollMutation;
        use crate::domain::zone::{item_sources, ItemInstance, ItemInstanceId, ItemLocation};
        let (zone, owner, _) = equipment_fixture();
        let baseline = encode_snapshot(&zone.snapshot()).unwrap();
        // Synthetic owners are built in a live zone and rebuilt there, so their combat
        // sheets follow the worn equipment and the envelope restores strictly.
        let mut live = zone.clone();
        let template = live.phase4.as_ref().unwrap().canonical.players[&owner.entity].clone();
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        // Distinct storable stackable templates (no quest items are admitted, so the D4
        // 100-ordinary + 100-quest owner is measured as 200 ordinary instances).
        let templates: Vec<u32> = rules
            .parts()
            .items
            .values()
            .filter(|d| {
                d.stackable
                    && rules
                        .parts()
                        .capabilities
                        .get(&d.template_id)
                        .is_some_and(|c| c.storable)
            })
            .map(|d| d.template_id)
            .take(200)
            .collect();
        assert_eq!(templates.len(), 200);
        // Unconditioned admitted equipment, tried in template order until no slot is free.
        let equipment: Vec<u32> = rules
            .parts()
            .items
            .values()
            .filter(|d| {
                matches!(d.kind.as_str(), "Weapon" | "Armor")
                    && !d.stackable
                    && d.conditions.is_empty()
                    && rules
                        .parts()
                        .capabilities
                        .get(&d.template_id)
                        .is_some_and(|c| c.admitted && c.storable)
            })
            .map(|d| d.template_id)
            .collect();
        // The widest admitted augmentation: the two highest option ids.
        let mut options = rules.parts().augment_options.keys().rev();
        let (high, low) = (*options.next().unwrap(), *options.next().unwrap());
        let augment = crate::domain::zone::item_refine::pack(low, high).unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state
        };
        // Maximal-width identities: a reserved block just below 2^62 (D8/D19 bound 2^63).
        let top = 1_u64 << 62;
        live.phase4.as_mut().unwrap().canonical.allocator =
            crate::domain::zone::ItemInstanceRange::new(1, top, top + (1 << 32)).unwrap();
        let mut allocated = 0_u64;
        for n in 0..8_u128 {
            let mut player = template.clone();
            player.inventory.clear();
            player.paperdoll.clear();
            player.equipment_sources.clear();
            player.active_sets.clear();
            player.auto_shots.clear();
            player.item_reuse.clear();
            let mut sequence = live
                .phase3
                .as_ref()
                .unwrap()
                .canonical
                .next_install_sequence;
            for template_id in &equipment {
                allocated += 1;
                let id = ItemInstanceId::new(top - allocated).unwrap();
                let mut item = template.inventory.values().next().unwrap().clone();
                item.id = id;
                item.template_id = *template_id;
                item.count = 1;
                item.location = ItemLocation::Inventory;
                item.slot = None;
                item.enchant = 0;
                item.augment_id = None;
                item.charges = 0;
                let mut staged = player.clone();
                staged.inventory.insert(id, item);
                let installed = PaperdollMutation::equip(
                    &staged.inventory,
                    &staged.paperdoll,
                    id,
                    None,
                    &rules,
                    staged.inventory_revision,
                )
                .ok()
                .filter(|m| m.paperdoll.len() > player.paperdoll.len())
                .and_then(|m| item_sources::apply_paperdoll(&staged, &m, &rules, sequence).ok());
                match installed {
                    Some((next_player, next_sequence)) => {
                        player = next_player;
                        sequence = next_sequence;
                    },
                    None => allocated -= 1,
                }
            }
            // Maximal-width equipped scalars: high enchant on every piece, the widest
            // augmentation on the weapon, then the enchant-dependent sets/providers.
            let worn: Vec<_> = player.paperdoll.values().copied().collect();
            for id in &worn {
                let item = player.inventory.get_mut(id).unwrap();
                item.enchant = 65_535;
                if item.slot == Some(crate::domain::zone::EquipSlot::Rhand) {
                    // Every charge bit (soulshot, spiritshot, blessed) is held by the weapon.
                    item.charges = 7;
                    item.augment_id = Some(augment);
                }
            }
            player.active_sets = item_sources::reconcile_sets(&player, &rules, None).unwrap();
            item_sources::reconcile_sources(&mut player, &rules, &mut sequence).unwrap();
            live.phase3
                .as_mut()
                .unwrap()
                .canonical
                .next_install_sequence = sequence;
            for template_id in &templates {
                allocated += 1;
                let id = ItemInstanceId::new(top - allocated).unwrap();
                let cap = if *template_id == 57 {
                    99_900_000_000
                } else {
                    i32::MAX as u64
                };
                player.inventory.insert(
                    id,
                    ItemInstance {
                        creation_origin: template
                            .inventory
                            .values()
                            .next()
                            .and_then(|i| i.creation_origin),
                        id,
                        template_id: *template_id,
                        count: (next() % cap).max(1),
                        location: ItemLocation::Inventory,
                        slot: None,
                        enchant: 0,
                        augment_id: None,
                        charges: 0,
                        created_tick: Tick(next() % 1_000_000),
                        revision: template.inventory_revision,
                    },
                );
            }
            // Each synthetic owner is a complete clone of the fixture player (entity, Phase3
            // sidecar) so the envelope encoder's cross-section validation still holds.
            let id = EntityId::from_uuid(Uuid::from_u128(0xC10_0000 + n));
            let mut entity = live.entities[&owner.entity].clone();
            entity.id = id;
            // Character names are ASCII letters only.
            entity.name = format!("Owner{}", char::from(b'A' + n as u8));
            if let Some(CombatRole::Player {
                progression: Some(p),
                ..
            }) = entity.combat.as_mut().map(|c| &mut c.role)
            {
                p.identity.account_id =
                    crate::domain::AccountId::from_uuid(Uuid::from_u128(0xACC0 + n));
            }
            live.aoi.insert(id, entity.pos);
            live.entities.insert(id, entity);
            let phase3 = &mut live.phase3.as_mut().unwrap().canonical;
            let mut sidecar = phase3.entities[&owner.entity].clone();
            sidecar.ordinal = crate::domain::zone::EntityOrdinal(phase3.next_entity_ordinal);
            phase3.next_entity_ordinal += 1;
            phase3.entities.insert(id, sidecar);
            let items = &mut live.phase4.as_mut().unwrap().canonical;
            items.players.insert(id, player);
            if let Some(facing) = items.facing.get(&owner.entity).copied() {
                items.facing.insert(id, facing);
            }
            live.rebuild_v5(id).unwrap();
        }
        let snapshot = live.snapshot();
        let encoded = encode_snapshot(&snapshot).unwrap();
        let decoded_len = u32::from_le_bytes(encoded[8..12].try_into().unwrap()) as usize;
        let worn: usize = snapshot
            .phase4
            .as_ref()
            .unwrap()
            .players
            .values()
            .map(|p| p.paperdoll.len())
            .sum();
        let instances: usize = snapshot
            .phase4
            .as_ref()
            .unwrap()
            .players
            .values()
            .map(|p| p.inventory.len())
            .sum();
        eprintln!(
            "PHASE4_C10 baseline_encoded={} worst_encoded={} worst_decoded={} owners={} instances={} worn={} budget_encoded={} budget_decoded={}",
            baseline.len(),
            encoded.len(),
            decoded_len,
            snapshot.phase4.as_ref().unwrap().players.len(),
            instances,
            worn,
            MAX_SNAPSHOT_ENCODED,
            MAX_SNAPSHOT_JSON
        );
        assert!(worn >= 8 * 12, "a full paperdoll per synthetic owner: {worn}");
        assert!(encoded.len() <= MAX_SNAPSHOT_ENCODED);
        assert!(decoded_len <= MAX_SNAPSHOT_JSON);
        // The worst envelope itself decodes and restores a runnable V6 zone.
        let decoded = decode_snapshot(&encoded).unwrap();
        assert_eq!(decoded, snapshot);
        ZoneState::from_snapshot_with_items(decoded, Some(Arc::clone(&rules))).unwrap();
        // Session-8 C10 population: the same worst-case owners in a 1,000-combatant zone
        // (the remaining combatants are template NPCs spread over the zone), one live tick.
        let npc_rules = load_rules(&RulesSource::embedded()).unwrap().rules;
        let template = parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0].clone();
        let combatants = live
            .entities
            .values()
            .filter(|e| e.combat.is_some())
            .count();
        let spawns: Vec<_> = (0..1000_usize.saturating_sub(combatants))
            .map(|n| {
                let n = i32::try_from(n).unwrap();
                ZoneInput::system(ZoneCommand::SpawnNpc {
                    name: "Keltir".into(),
                    pos: Vec2Fixed::from_tiles(20 + (n % 200), 20 + (n / 200) * 40),
                    speed: template.move_speed,
                    combat: Some(Box::new(
                        NpcCombat::from_template(&npc_rules, &template).unwrap(),
                    )),
                })
            })
            .collect();
        assert!(live
            .run_tick(live.draft(spawns))
            .unwrap()
            .dispositions
            .is_empty());
        let mut population = live.snapshot();
        // Review-5 P2-3: before the pools are injected, the 1,000-combatant envelope with the
        // worst-case owners encodes, decodes and restores into a runnable V6 zone.
        {
            let encoded = encode_snapshot(&population).unwrap();
            let decoded = decode_snapshot(&encoded).unwrap();
            assert_eq!(decoded, population);
            let mut cold =
                ZoneState::from_snapshot_with_items(decoded, Some(Arc::clone(&rules))).unwrap();
            cold.run_tick(cold.draft(Vec::new())).unwrap();
        }
        // D106: full effect pools (24 buffs, 12 songs, 8 debuffs = 44 each) on every NPC, as
        // benches/support/snapshot_capacity.rs builds them, combined with the worst-case
        // owners: 44 synthetic, independently stackable periodic skills join the epoch rules.
        {
            use crate::domain::zone::effects::{
                compile_periodic, ActiveEffect, ActiveHandler, ActivePeriodic, EffectFlags,
                EffectId, EffectList, EffectPool, EffectSource, EffectSourceKind, PulseSchedule,
            };
            use crate::domain::zone::skill::{SkillId, SkillRef};
            let phase = population.phase3.as_mut().unwrap();
            // V6 envelopes reference the epoch rules by hash (D27), so synthetic skills
            // cannot join them; every pool entry reuses the source-valid periodic skill 1095
            // with the bench's independent "NONE" abnormal stacking, which is what the
            // encoded size depends on.
            let rules = Arc::clone(&phase.rules);
            // 44 distinct admitted abnormal skills, periodic ones first (largest handlers).
            let mut pool: Vec<SkillId> = rules
                .skills()
                .skills()
                .values()
                .filter(|s| {
                    s.abnormal.is_some()
                        && s.effects.first().is_some_and(|h| {
                            compile_periodic(h, 1, s.max_level).is_ok_and(|p| p.is_some())
                        })
                })
                .map(|s| s.id)
                .collect();
            let rest: Vec<SkillId> = rules
                .skills()
                .skills()
                .values()
                .filter(|s| s.abnormal.is_some() && !pool.contains(&s.id))
                .map(|s| s.id)
                .collect();
            pool.extend(rest);
            pool.truncate(44);
            assert_eq!(pool.len(), 44);
            let npcs: Vec<_> = population
                .entities
                .iter()
                .filter(|e| e.kind == EntityKind::Npc)
                .map(|e| e.id)
                .collect();
            let mut next_effect = phase.next_effect_id;
            let mut sequence = phase.next_install_sequence;
            for id in npcs {
                let state = phase.entities.get_mut(&id).unwrap();
                let mut entries = state.effects.entries().clone();
                for offset in 0..44_u32 {
                    let skill = rules
                        .skills()
                        .get(pool[usize::try_from(offset).unwrap()])
                        .unwrap();
                    let abnormal = skill.abnormal.as_ref().unwrap();
                    let periodic = skill
                        .effects
                        .first()
                        .and_then(|h| compile_periodic(h, 1, skill.max_level).ok().flatten());
                    let effect = ActiveEffect {
                        id: EffectId(next_effect),
                        skill: SkillRef {
                            id: skill.id,
                            rank: 1,
                        },
                        source: EffectSource {
                            binding: Some(TargetBinding {
                                entity: EntityId::from_uuid(Uuid::from_u128(
                                    0x00E0_0000_0000 + u128::from(next_effect),
                                )),
                                incarnation: 1,
                            }),
                            kind: EffectSourceKind::Player,
                        },
                        target_incarnation: 1,
                        applied_at: population.tick,
                        expires_at: Some(Tick(population.tick.0 + 12_000)),
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
                            stay_after_death: *skill
                                .stay_after_death
                                .at(1, skill.max_level)
                                .unwrap(),
                            stay_on_subclass_change: *skill
                                .stay_on_subclass_change
                                .at(1, skill.max_level)
                                .unwrap(),
                            removed_on_damage: *skill
                                .removed_on_damage
                                .at(1, skill.max_level)
                                .unwrap(),
                            removed_on_action_except_move: *skill
                                .removed_on_action_except_move
                                .at(1, skill.max_level)
                                .unwrap(),
                            irreplaceable: *skill
                                .irreplaceable_buff
                                .at(1, skill.max_level)
                                .unwrap(),
                        },
                        handlers: vec![ActiveHandler {
                            xml_ordinal: 0,
                            periodic: periodic.map(|(resource, power, interval)| ActivePeriodic {
                                resource,
                                power,
                                schedule: PulseSchedule::new(population.tick, interval).unwrap(),
                            }),
                        }],
                    };
                    state
                        .modifier_install_order
                        .insert(sequence, RestoredModifierSource::Effect(effect.id));
                    entries.insert(effect.id, effect);
                    next_effect += 1;
                    sequence += 1;
                }
                state.effects = EffectList::from_entries(entries).unwrap();
            }
            phase.next_effect_id = next_effect;
            phase.next_install_sequence = sequence;
        }
        assert_eq!(
            population
                .entities
                .iter()
                .filter(|e| e.combat.is_some())
                .count(),
            1000
        );
        let encoded = encode_snapshot(&population).unwrap();
        let decoded_len = u32::from_le_bytes(encoded[8..12].try_into().unwrap()) as usize;
        let effects: usize = population
            .phase3
            .as_ref()
            .unwrap()
            .entities
            .values()
            .map(|e| e.effects.entries().len())
            .sum();
        eprintln!(
            "PHASE4_C10_POPULATION combatants=1000 effects={effects} item_owners={} encoded={} decoded={} budget_encoded={} budget_decoded={}",
            population.phase4.as_ref().unwrap().players.len(),
            encoded.len(),
            decoded_len,
            MAX_SNAPSHOT_ENCODED,
            MAX_SNAPSHOT_JSON
        );
        assert!(encoded.len() <= MAX_SNAPSHOT_ENCODED);
        assert!(decoded_len <= MAX_SNAPSHOT_JSON);
        assert!(effects >= 990 * 44);
        // The pools are injected into the snapshot (their stat effects are not folded into
        // the NPC sheets), so this envelope is a size/codec gate: it decodes exactly. The
        // worst-case owners alone restored through from_snapshot_with_items above.
        let decoded = decode_snapshot(&encoded).unwrap();
        assert_eq!(decoded, population);
    }

    /// Recorded evidence run: every tick is cold-replayed from the boundary, its record kept
    /// and its D46 facts collected; `write` emits the exact recording for the CLI exporter.
    struct EvidenceRun {
        zone: ZoneState,
        cold: ZoneState,
        boundary: crate::domain::zone::ZoneSnapshot,
        records: Vec<crate::application::replay_log::AppliedTickRecord>,
        facts: Vec<crate::domain::zone::evidence::ItemEvidenceFact>,
        seq: u32,
    }
    impl EvidenceRun {
        fn start(mut zone: ZoneState) -> Self {
            let boundary = zone.snapshot();
            let cold = ZoneState::from_snapshot_with_items(
                boundary.clone(),
                Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
            )
            .unwrap();
            zone.enable_items_replay_evidence().unwrap();
            Self {
                zone,
                cold,
                boundary,
                records: Vec::new(),
                facts: Vec::new(),
                seq: 100,
            }
        }
        /// Session commands from `owner`, in order.
        fn tick(&mut self, owner: EntityId, commands: Vec<ZoneCommand>) -> AppliedTick {
            let inputs = commands
                .into_iter()
                .map(|command| {
                    self.seq = self.seq.checked_add(1).unwrap();
                    ZoneInput::session(owner, SessionGeneration(1), self.seq, command)
                })
                .collect();
            let draft = self.zone.draft(inputs);
            let applied = self.zone.run_tick(draft.clone()).unwrap();
            assert_eq!(applied, self.cold.run_tick(draft).unwrap());
            self.records
                .push(crate::application::replay_log::AppliedTickRecord::from_applied(
                    self.zone.seed.zone,
                    &applied,
                ));
            self.facts.extend(self.zone.take_items_replay_evidence());
            applied
        }
        fn formulas(&self) -> std::collections::BTreeSet<String> {
            self.facts
                .iter()
                .filter_map(|f| match &f.fact {
                    crate::domain::zone::evidence::ItemEvidenceData::Formula(v) => {
                        v["formula"].as_str().map(str::to_owned)
                    },
                    _ => None,
                })
                .collect()
        }
        fn write(self, variable: &str) {
            use crate::application::replay_log::{Watermark, WatermarkReason};
            if let Ok(path) = std::env::var(variable) {
                let recording = crate::infrastructure::eventlog::Recording {
                    watermark: Watermark {
                        zone: self.boundary.seed.zone,
                        epoch: self.boundary.seed.epoch,
                        records: u64::try_from(self.records.len()).unwrap(),
                        last_tick: self.records.last().map(|r| r.tick),
                        reason: WatermarkReason::Shutdown,
                    },
                    records: self.records,
                    snapshot: self.boundary,
                };
                std::fs::write(path, recording.to_bytes().unwrap()).unwrap();
            }
        }
    }

    /// D148: three toggle ticks for the exporter: toggle-on of 1463 (D grade on Flamberge:
    /// no charge, a pure step), toggle-off of 1463 (a pure step), then toggle-on of 1464
    /// (charges the weapon: a Use row under its key, so no pure-toggle delta). Each pure
    /// toggle emits one instance-less `toggle_auto_shot` delta at the settled revision.
    #[test]
    fn v6_pure_toggle_steps_export_a_toggle_delta() {
        use crate::domain::zone::ItemCommandKind as K;
        let (mut zone, owner, _, _) = use_fixture(&[1463, 1464]);
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut run = EvidenceRun::start(zone);
        let toggle = |run: &mut EvidenceRun, key: u128, template: u32, enabled: bool| {
            let intent =
                item_intent(&run.zone, owner.entity, key, K::ToggleAutoShot { template, enabled });
            let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(intent))]);
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        };
        toggle(&mut run, 3401, 1463, true);
        toggle(&mut run, 3402, 1463, false);
        toggle(&mut run, 3403, 1464, true);
        let steps: Vec<_> = run
            .facts
            .iter()
            .filter_map(|f| match &f.fact {
                crate::domain::zone::evidence::ItemEvidenceData::Delta(v)
                    if v["operation"] == "toggle_auto_shot" =>
                {
                    Some(v.clone())
                },
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert_eq!(
            (&steps[0]["template"], &steps[0]["enabled"]),
            (&serde_json::json!(1463), &serde_json::json!(true))
        );
        assert_eq!(
            (&steps[1]["template"], &steps[1]["enabled"]),
            (&serde_json::json!(1463), &serde_json::json!(false))
        );
        assert!(steps
            .iter()
            .all(|s| s.get("instance").is_none() && s["count_delta"] == 0));
        assert_eq!(
            steps[1]["inventory_revision"].as_u64().unwrap(),
            steps[0]["inventory_revision"].as_u64().unwrap() + 1
        );
        run.write("NIGHTFALL_TEST_TOGGLE_STEPS_RECORDING");
    }

    #[test]
    fn v6_bow_evidence_recording_exports_damage_reuse_and_launch() {
        use crate::domain::zone::ItemCommandKind as K;
        let (mut zone, owner, npc, ids) = use_fixture(&[13]);
        let start = zone.entities[&owner.entity].pos;
        place(
            &mut zone,
            npc.entity,
            Vec2Fixed::new(Fixed::from_raw(start.x.raw() + 6_000), start.y),
        );
        let mut run = EvidenceRun::start(zone);
        let equip = item_intent(
            &run.zone,
            owner.entity,
            1301,
            K::Equip {
                instance: ids[0],
                preferred_slot: None,
            },
        );
        run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(equip))]);
        run.tick(
            owner.entity,
            vec![
                ZoneCommand::SetTarget {
                    entity: owner.entity,
                    target: Some(npc.entity),
                },
                ZoneCommand::Attack {
                    entity: owner.entity,
                },
            ],
        );
        for _ in 0..60 {
            run.tick(owner.entity, Vec::new());
        }
        let names = run.formulas();
        for required in ["bow_reuse", "bow_damage", "bow_launch", "physical_damage"] {
            assert!(names.contains(required), "{required}: {names:?}");
        }
        run.write("NIGHTFALL_TEST_BOW_RECORDING");
    }

    #[test]
    fn v6_gear_evidence_recording_exports_set_shield_history_and_clamps() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        // Chest 58 and Hoplon 628 belong to set 3; Phoenix Necklace 933 adds maxMp 26.
        let (zone, owner, _, ids) = use_fixture(&[58, 628, 933]);
        let mut run = EvidenceRun::start(zone);
        let mut request = 1400_u128;
        let mut step = |run: &mut EvidenceRun, kind: K| {
            request += 1;
            let intent = item_intent(&run.zone, owner.entity, request, kind);
            let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(intent))]);
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        };
        let equip = |instance| K::Equip {
            instance,
            preferred_slot: None,
        };
        // The fixture wears the full set; break it (legs) and take the chest off first so the
        // 4-armor-set phases see an incomplete set.
        step(
            &mut run,
            K::Unequip {
                slot: EquipSlot::Legs,
            },
        ); // remove of a required part
        step(
            &mut run,
            K::Unequip {
                slot: EquipSlot::Chest,
            },
        ); // no chest: no set transition
        step(&mut run, equip(ids[1])); // shield first: no chest, no set transition
        step(&mut run, equip(ids[0])); // shield-then-chest: equip_chest, incomplete, false
        step(
            &mut run,
            K::Unequip {
                slot: EquipSlot::Lhand,
            },
        ); // remove
        step(&mut run, equip(ids[1])); // chest-then-shield: equip_shield, incomplete, true
        step(&mut run, equip(ids[2])); // maxMp +26: clamp_no_heal (no heal)
        step(
            &mut run,
            K::Unequip {
                slot: EquipSlot::Neck,
            },
        ); // maxMp −26: a lowered maximum clamps
        let names = run.formulas();
        for required in ["set_shield_history", "clamp_no_heal"] {
            assert!(names.contains(required), "{required}: {names:?}");
        }
        run.write("NIGHTFALL_TEST_GEAR_RECORDING");
    }

    #[test]
    fn v6_heal_evidence_recording_exports_heal() {
        // Heal 1011 rank 1 on the wounded caster (TARGET), from a real learned skill.
        let (mut zone, owner, _, _) = use_fixture(&[]);
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
            key: "l2.skill.1011".into(),
            level: 1,
        }];
        zone.rebuild_v5(owner.entity).unwrap();
        set_resources(&mut zone, owner, 10, 0);
        let mut run = EvidenceRun::start(zone);
        run.tick(
            owner.entity,
            vec![ZoneCommand::CastSkill {
                entity: owner.entity,
                skill_id: crate::domain::zone::skill::SkillId(1011),
                target: Some(owner.entity),
            }],
        );
        for _ in 0..70 {
            run.tick(owner.entity, Vec::new());
        }
        assert!(run.formulas().contains("heal"), "{:?}", run.formulas());
        run.write("NIGHTFALL_TEST_HEAL_RECORDING");
    }

    #[test]
    fn v6_consumable_evidence_recording_exports_consumable_use() {
        use crate::domain::zone::ItemCommandKind as K;
        // Lesser Healing Potion 1060 (admitted under D97) used by a wounded owner.
        let (mut zone, owner, _, ids) = use_fixture(&[1060]);
        set_resources(&mut zone, owner, 10, 0);
        let mut run = EvidenceRun::start(zone);
        let intent = item_intent(
            &run.zone,
            owner.entity,
            1501,
            K::Use {
                instance: ids[0],
                target: None,
            },
        );
        let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(intent))]);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        for _ in 0..30 {
            run.tick(owner.entity, Vec::new());
        }
        assert!(run.formulas().contains("consumable_use"), "{:?}", run.formulas());
        run.write("NIGHTFALL_TEST_CONSUMABLE_RECORDING");
    }

    #[test]
    fn v6_escape_evidence_recording_exports_escape_timing() {
        use crate::domain::zone::ItemCommandKind as K;
        // Scroll of Escape 736: prepaid at start, one fact on the teleport landing tick.
        let (zone, owner, _, ids) = use_fixture(&[736]);
        let zone = zone.with_safe_point(Vec2Fixed::from_tiles(126, 126));
        let mut run = EvidenceRun::start(zone);
        let intent = item_intent(
            &run.zone,
            owner.entity,
            1601,
            K::Use {
                instance: ids[0],
                target: None,
            },
        );
        let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(intent))]);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        for _ in 0..260 {
            run.tick(owner.entity, Vec::new());
            if matches!(run.zone.sidecar(owner.entity).unwrap().cast, CastState::Idle) {
                break;
            }
        }
        run.tick(owner.entity, Vec::new());
        let names = run.formulas();
        assert!(names.contains("escape_timing"), "{names:?}");
        run.write("NIGHTFALL_TEST_ESCAPE_RECORDING");
    }

    /// E4.6 scenario support (4-combat-kinds blow/lethal phases): a level-40 Treasure Hunter
    /// with Mortal Blow 16:24 and Deadly Blow 263:3 casts from behind a stationary target
    /// facing east with dagger 10; the export carries `blow_success` and `lethal` facts with the
    /// scenario's skill contexts.
    #[test]
    #[allow(clippy::too_many_lines)] // Class/level fixture, equip, two casts and the export.
    fn v6_scenario_blow_and_lethal_from_behind_with_a_dagger() {
        use crate::domain::subclass::LearnedSkill;
        use crate::domain::zone::skill::SkillId;
        use crate::domain::zone::ItemCommandKind as K;
        let (mut zone, owner, npc, ids) = use_fixture(&[10]);
        let xp_at_40 = zone.rules.as_ref().unwrap().xp_to_level(40).unwrap();
        let base_stats = zone
            .classes
            .as_ref()
            .unwrap()
            .get(ClassId(0))
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
            class,
            xp,
            progression: Some(player),
        } = &mut combat.role
        else {
            unreachable!()
        };
        // The shared fixture spawns Human Mystics; this owner is re-based as a Human Fighter.
        *class = "human_fighter".into();
        player.identity.base_class_id = ClassId(0);
        player.class_state = ClassState::new(ClassId(0));
        *xp = xp_at_40;
        player.class_state.current_class_id = ClassId(8);
        // Human Fighter → Rogue (20) → Treasure Hunter (40), with both transfer receipts.
        for (target_class, level) in [(7, 20), (8, 40)] {
            player.class_state.successful_transfer_receipts.push(
                crate::domain::character_progression::SuccessfulTransferReceipt {
                    key: Uuid::from_u128(u128::from(target_class) + 0xB10),
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
        player.class_state.merge_learned_skills(vec![
            LearnedSkill {
                key: "l2.skill.16".into(),
                level: 24,
            },
            LearnedSkill {
                key: "l2.skill.263".into(),
                level: 3,
            },
        ]);
        zone.relevel(owner.entity).unwrap();
        // The fighter line's collision radius follows the re-based profession.
        let radius = {
            let c = zone.entities[&owner.entity].combat.as_ref().unwrap();
            let CombatRole::Player {
                progression: Some(p),
                ..
            } = &c.role
            else {
                unreachable!()
            };
            crate::domain::zone::profession_stats(
                zone.rules.as_ref().unwrap(),
                zone.classes.as_ref().unwrap(),
                &p.identity,
                &p.class_state,
                c.sheet.level(),
            )
            .unwrap()
            .3
        };
        zone.entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .collision_radius = radius;
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
        // Target at (108,100) facing east; the owner one tile west is behind it.
        place(&mut zone, npc.entity, Vec2Fixed::from_tiles(108, 100));
        place(&mut zone, owner.entity, Vec2Fixed::from_tiles(107, 100));
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut run = EvidenceRun::start(zone);
        let equip = item_intent(
            &run.zone,
            owner.entity,
            1701,
            K::Equip {
                instance: ids[0],
                preferred_slot: None,
            },
        );
        let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(equip))]);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        for skill in [16, 263] {
            let applied = run.tick(
                owner.entity,
                vec![ZoneCommand::CastSkill {
                    entity: owner.entity,
                    skill_id: SkillId(skill),
                    target: Some(npc.entity),
                }],
            );
            assert!(applied.dispositions.is_empty(), "{skill}: {:?}", applied.dispositions);
            for _ in 0..40 {
                run.tick(owner.entity, Vec::new());
                if matches!(run.zone.sidecar(owner.entity).unwrap().cast, CastState::Idle) {
                    break;
                }
            }
        }
        let names = run.formulas();
        for required in ["blow_success", "lethal"] {
            assert!(names.contains(required), "{required}: {names:?}");
        }
        run.write("NIGHTFALL_TEST_SCENARIO_BLOW_RECORDING");
    }

    #[test]
    fn v6_resurrection_evidence_recording_exports_resurrection_scroll() {
        use crate::domain::zone::ItemCommandKind as K;
        // D99/D104: Scroll of Resurrection 737 on a dead peer; one fact on the target when it
        // accepts, under the caster's use receipt.
        let (mut zone, owner, npc, ids) = use_fixture(&[737]);
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
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut run = EvidenceRun::start(zone);
        let intent = item_intent(
            &run.zone,
            owner.entity,
            1801,
            K::Use {
                instance: ids[0],
                target: None,
            },
        );
        let applied = run.tick(
            owner.entity,
            vec![
                ZoneCommand::SetTarget {
                    entity: owner.entity,
                    target: Some(dead),
                },
                ZoneCommand::Item(Box::new(intent)),
            ],
        );
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let mut offer = None;
        for _ in 0..200 {
            run.tick(owner.entity, Vec::new());
            offer = run
                .zone
                .sidecar(dead)
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .resurrect_offer
                .as_ref()
                .map(|o| o.id);
            if offer.is_some() {
                break;
            }
        }
        // Engine-review-14 P3-2: the dead life faces north-west; the accepted
        // resurrection starts the new life east (T51/D157).
        let facing = |zone: &ZoneState| zone.phase4.as_ref().unwrap().canonical.facing[&dead];
        let point = |x, y| crate::domain::zone::Vec2Fixed {
            x: crate::domain::zone::Fixed::from_raw(x),
            y: crate::domain::zone::Fixed::from_raw(y),
        };
        let east = crate::domain::zone::item_angles::FacingVector::EAST;
        *run.zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .facing
            .get_mut(&dead)
            .unwrap() = east.moved(point(5, 5), point(3, 3));
        assert_ne!(facing(&run.zone), east);
        let applied = run.tick(
            dead,
            vec![ZoneCommand::AcceptResurrect {
                entity: dead,
                offer_id: offer.unwrap(),
                accept: true,
            }],
        );
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert!(!run.zone.entities[&dead].targeting.dead);
        assert_eq!(facing(&run.zone), east);
        assert!(run.formulas().contains("resurrection_scroll"), "{:?}", run.formulas());
        // D139: the subject is the scroll's owner; the resurrected player is the context's
        // `target_entity`.
        let fact = run
            .facts
            .iter()
            .find(|f| {
                matches!(&f.fact, crate::domain::zone::evidence::ItemEvidenceData::Formula(v)
                if v["formula"] == "resurrection_scroll")
            })
            .unwrap();
        assert_eq!(fact.entity, owner.entity);
        let crate::domain::zone::evidence::ItemEvidenceData::Formula(value) = &fact.fact else {
            unreachable!()
        };
        assert_eq!(value["context"]["target_entity"], dead.to_string());
        run.write("NIGHTFALL_TEST_RESURRECTION_RECORDING");
    }

    #[test]
    fn v6_d94_shapes_recording_exports_through_the_evidence_exporter() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        // Review-4 P1: [C, T] (Enchant then Destroy the scroll stack) and [Dp, C] (partial
        // Destroy then Enchant on one scroll stack) in real ticks.
        let (mut zone, owner, target, scroll) = enchant_fixture(956, EquipSlot::Chest, 0);
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .get_mut(&scroll)
            .unwrap()
            .count = 6;
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut run = EvidenceRun::start(zone);
        let mut request = 1900_u128;
        let mut intent = |run: &EvidenceRun, kind: K| {
            request += 1;
            ZoneCommand::Item(Box::new(item_intent(&run.zone, owner.entity, request, kind)))
        };
        let enchant = K::Enchant {
            target,
            scroll,
            support: None,
        };
        let first = vec![
            intent(
                &run,
                K::Destroy {
                    instance: scroll,
                    count: 1,
                },
            ),
            intent(&run, enchant.clone()),
        ];
        let applied = run.tick(owner.entity, first);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let second = vec![
            intent(&run, enchant),
            intent(
                &run,
                K::Destroy {
                    instance: scroll,
                    count: 3,
                },
            ),
        ];
        let applied = run.tick(owner.entity, second);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        run.write("NIGHTFALL_TEST_D94_RECORDING");
    }

    /// D138 (T31): an equip of an enchanted, grade-penalized armour piece still emits its
    /// `gear_stats` install fact, with the winning grade-penalty provider (6213) folded and
    /// reported; its unequip emits one too.
    #[test]
    fn v6_gear_stats_covers_enchanted_and_penalized_installs() {
        use crate::domain::zone::{item_sources, EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, _, ids) = use_fixture(&[10491]);
        {
            let items = zone.phase4.as_mut().unwrap();
            let rules = Arc::clone(&items.rules);
            let player = items.canonical.players.get_mut(&owner.entity).unwrap();
            player.inventory.retain(|id, _| ids.contains(id));
            for item in player.inventory.values_mut() {
                item.slot = None;
                item.location = crate::domain::zone::ItemLocation::Inventory;
                item.charges = 0;
                item.enchant = 4;
            }
            player.paperdoll.clear();
            player.auto_shots.clear();
            player.active_sets = item_sources::reconcile_sets(player, &rules, None).unwrap();
            let next = &mut zone
                .phase3
                .as_mut()
                .unwrap()
                .canonical
                .next_install_sequence;
            item_sources::reconcile_sources(player, &rules, next).unwrap();
        }
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut run = EvidenceRun::start(zone);
        let gear = |run: &EvidenceRun, resolution: &str| {
            run.facts
                .iter()
                .filter_map(|f| match &f.fact {
                    crate::domain::zone::evidence::ItemEvidenceData::Formula(v)
                        if v["formula"] == "gear_stats" && v["resolution_id"] == resolution =>
                    {
                        Some((f.entity, v.clone()))
                    },
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let equip = item_intent(
            &run.zone,
            owner.entity,
            2701,
            K::Equip {
                instance: ids[0],
                preferred_slot: None,
            },
        );
        let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(equip))]);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        let held = &run.zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(held.armor_penalty > 0, "the C-grade shirt is penalized");
        assert_eq!(held.inventory[&ids[0]].enchant, 4);
        let installed = gear(&run, &format!("equip:{}", Uuid::from_u128(2701)));
        assert_eq!(installed.len(), 1, "{:?}", run.formulas());
        assert_eq!(installed[0].0, owner.entity);
        let fact = &installed[0].1;
        assert_eq!(fact["context"]["event"], "equip");
        assert!(fact["inputs"]["equipment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[0] == 10491 && row[1] == 4));
        assert!(fact["inputs"]["skill_refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[0] == 6213 && row[1] == held.armor_penalty));
        let unequip = item_intent(
            &run.zone,
            owner.entity,
            2702,
            K::Unequip {
                slot: EquipSlot::Under,
            },
        );
        let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(unequip))]);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(gear(&run, &format!("unequip:{}", Uuid::from_u128(2702))).len(), 1);
        run.write("NIGHTFALL_TEST_GEAR_PENALTY_RECORDING");
    }

    #[test]
    fn v6_gear_stats_recording_exports_engine_folds() {
        use crate::domain::zone::{item_sources, EquipSlot, ItemCommandKind as K};
        // D103(a): a light owner (only no-grade gear, so no grade/overload penalty) equips and
        // unequips a weapon and a shirt; each rebuild exports gear_stats.
        let (mut zone, owner, _, ids) = use_fixture(&[1, 21]);
        {
            let items = zone.phase4.as_mut().unwrap();
            let rules = Arc::clone(&items.rules);
            let player = items.canonical.players.get_mut(&owner.entity).unwrap();
            player.inventory.retain(|id, _| ids.contains(id));
            for item in player.inventory.values_mut() {
                item.slot = None;
                item.location = crate::domain::zone::ItemLocation::Inventory;
                item.charges = 0;
            }
            player.paperdoll.clear();
            player.auto_shots.clear();
            player.active_sets = item_sources::reconcile_sets(player, &rules, None).unwrap();
            let next = &mut zone
                .phase3
                .as_mut()
                .unwrap()
                .canonical
                .next_install_sequence;
            item_sources::reconcile_sources(player, &rules, next).unwrap();
        }
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let held = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!((held.weapon_penalty, held.armor_penalty, held.overload_level), (0, 0, 0));
        let mut run = EvidenceRun::start(zone);
        let mut request = 2000_u128;
        let mut step = |run: &mut EvidenceRun, kind: K| {
            request += 1;
            let intent = item_intent(&run.zone, owner.entity, request, kind);
            let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(intent))]);
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        };
        for id in &ids {
            step(
                &mut run,
                K::Equip {
                    instance: *id,
                    preferred_slot: None,
                },
            );
        }
        step(
            &mut run,
            K::Unequip {
                slot: EquipSlot::Rhand,
            },
        );
        assert!(run.formulas().contains("gear_stats"), "{:?}", run.formulas());
        run.write("NIGHTFALL_TEST_GEAR_STATS_RECORDING");
    }

    #[test]
    fn v6_reduced_mp_bow_recording_tabulates_the_reduction_draw() {
        use crate::domain::zone::ItemCommandKind as K;
        // D109: bow 4818 (reduced_mp_consume 47,1) with Steel Arrows 1342; the reduced-MP draw
        // is an item_rng fact, so the export's gap-free cursor holds.
        let (mut zone, owner, npc, ids) = use_fixture(&[4818, 1342]);
        let start = zone.entities[&owner.entity].pos;
        place(
            &mut zone,
            npc.entity,
            Vec2Fixed::new(Fixed::from_raw(start.x.raw() + 6_000), start.y),
        );
        let mut run = EvidenceRun::start(zone);
        let equip = item_intent(
            &run.zone,
            owner.entity,
            2101,
            K::Equip {
                instance: ids[0],
                preferred_slot: None,
            },
        );
        run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(equip))]);
        run.tick(
            owner.entity,
            vec![
                ZoneCommand::SetTarget {
                    entity: owner.entity,
                    target: Some(npc.entity),
                },
                ZoneCommand::Attack {
                    entity: owner.entity,
                },
            ],
        );
        for _ in 0..40 {
            run.tick(owner.entity, Vec::new());
        }
        assert!(run.facts.iter().any(|f| matches!(&f.fact,
            crate::domain::zone::evidence::ItemEvidenceData::Rng { callsite, .. }
                if callsite == "bow_reduced_mp")));
        run.write("NIGHTFALL_TEST_REDUCED_BOW_RECORDING");
    }

    /// E4.6 scenario support: 4-combat-kinds `unarmed` (`fist_spread` on weapon 0) and
    /// `pole-primary` (`pole_damage` primary) phases from real swings.
    #[test]
    fn v6_scenario_unarmed_and_pole_primary_recording() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (zone, owner, npc, ids) = use_fixture(&[15]);
        let mut run = EvidenceRun::start(zone);
        let mut request = 2200_u128;
        let mut item = |run: &mut EvidenceRun, kind: K| {
            request += 1;
            let intent = item_intent(&run.zone, owner.entity, request, kind);
            let applied = run.tick(owner.entity, vec![ZoneCommand::Item(Box::new(intent))]);
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        };
        if run.zone.phase4.as_ref().unwrap().canonical.players[&owner.entity]
            .paperdoll
            .contains_key(&EquipSlot::Rhand)
        {
            item(
                &mut run,
                K::Unequip {
                    slot: EquipSlot::Rhand,
                },
            );
        }
        let attack = |run: &mut EvidenceRun| {
            run.tick(
                owner.entity,
                vec![
                    ZoneCommand::SetTarget {
                        entity: owner.entity,
                        target: Some(npc.entity),
                    },
                    ZoneCommand::Attack {
                        entity: owner.entity,
                    },
                ],
            );
            for _ in 0..40 {
                run.tick(owner.entity, Vec::new());
            }
        };
        attack(&mut run);
        item(
            &mut run,
            K::Equip {
                instance: ids[0],
                preferred_slot: None,
            },
        );
        attack(&mut run);
        let names = run.formulas();
        for required in ["fist_spread", "pole_damage"] {
            assert!(names.contains(required), "{required}: {names:?}");
        }
        run.write("NIGHTFALL_TEST_SCENARIO_KINDS_RECORDING");
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
            phase5: None,
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
    #[allow(clippy::too_many_lines)] // Actual equip, start guards, payment, timing and falloff.
    fn v6_bow_pays_arrow_and_mp_at_start_uses_bow_timing_reach_and_distance_falloff() {
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{EquipSlot, ItemCommandKind};
        let (mut zone, owner, npc, ids) = use_fixture(&[13]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let bow = ids[0];
        let equip = item_intent(
            &zone,
            owner.entity,
            983,
            ItemCommandKind::Equip {
                instance: bow,
                preferred_slot: None,
            },
        );
        let tick = zone
            .run_tick(zone.draft(vec![review_input(
                owner.entity,
                1,
                ZoneCommand::Item(Box::new(equip)),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let held = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        let arrow = held.paperdoll[&EquipSlot::Lhand];
        assert_eq!(held.inventory[&arrow].template_id, 17);
        let stock = held.inventory[&arrow].count;
        // Bow reach is floor(500×1000/32) milli-tiles (CharStat.java:424-436, design §5.4).
        assert_eq!(
            zone.entities[&owner.entity]
                .combat
                .as_ref()
                .unwrap()
                .attack_range,
            Fixed::from_raw(15_625)
        );
        // Eight tiles away: far outside melee reach, inside bow reach.
        let start = zone.entities[&owner.entity].pos;
        let far = Vec2Fixed::new(Fixed::from_raw(start.x.raw() + 8_000), start.y);
        let old = zone.entities[&npc.entity].pos;
        zone.entities.get_mut(&npc.entity).unwrap().pos = far;
        zone.aoi.relocate(npc.entity, old, far);
        let attack = |zone: &ZoneState, seq| {
            zone.draft(vec![
                review_input(
                    owner.entity,
                    seq,
                    ZoneCommand::SetTarget {
                        entity: owner.entity,
                        target: Some(npc.entity),
                    },
                ),
                review_input(
                    owner.entity,
                    seq + 1,
                    ZoneCommand::Attack {
                        entity: owner.entity,
                    },
                ),
            ])
        };
        // Not enough MP: no start, no arrow, the intention is retained (retry later).
        let mut starved = zone.clone();
        starved
            .entities
            .get_mut(&owner.entity)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .mp = 0;
        starved.sidecar_mut(owner.entity).unwrap().fractions.mp = 0;
        let waited = starved.run_tick(attack(&starved, 2)).unwrap();
        assert!(!waited
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::AttackStarted { .. })));
        let c = starved.entities[&owner.entity].combat.as_ref().unwrap();
        assert!(c.auto_attack && c.swing.is_none());
        assert_eq!(
            starved.phase4.as_ref().unwrap().canonical.players[&owner.entity].inventory[&arrow]
                .count,
            stock
        );
        // No arrows anywhere: the attack intention ends without payment.
        let mut empty = zone.clone();
        let destroy = item_intent(
            &empty,
            owner.entity,
            984,
            ItemCommandKind::Destroy {
                instance: arrow,
                count: stock,
            },
        );
        let destroyed = empty
            .run_tick(empty.draft(vec![review_input(
                owner.entity,
                2,
                ZoneCommand::Item(Box::new(destroy)),
            )]))
            .unwrap();
        assert!(destroyed.dispositions.is_empty(), "{:?}", destroyed.dispositions);
        assert!(!empty.phase4.as_ref().unwrap().canonical.players[&owner.entity]
            .inventory
            .values()
            .any(|item| item.template_id == 17));
        let mp = empty.entities[&owner.entity].combat.as_ref().unwrap().mp;
        let stopped = empty.run_tick(attack(&empty, 3)).unwrap();
        assert!(!stopped
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::AttackStarted { .. })));
        let c = empty.entities[&owner.entity].combat.as_ref().unwrap();
        assert!(!c.auto_attack);
        assert_eq!(c.mp, mp);
        // The last arrow fires and leaves the left hand empty (design §5.4).
        let mut last = zone.clone();
        let trim = item_intent(
            &last,
            owner.entity,
            985,
            ItemCommandKind::Destroy {
                instance: arrow,
                count: stock - 1,
            },
        );
        let trimmed = last
            .run_tick(last.draft(vec![review_input(
                owner.entity,
                2,
                ZoneCommand::Item(Box::new(trim)),
            )]))
            .unwrap();
        assert!(trimmed.dispositions.is_empty(), "{:?}", trimmed.dispositions);
        let last_checkpoint = last
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, last.next_tick, &last.phase3.as_ref().unwrap().canonical)
            .unwrap();
        let shot = last.run_tick(attack(&last, 3)).unwrap();
        // T26 (4-bow-last-arrow): the last arrow's debit and the emptied left hand settle.
        crate::application::ports::ItemCheckpointWork {
            zone: last.seed.zone,
            epoch: last.seed.epoch,
            allocation: last.phase4.as_ref().unwrap().canonical.allocator,
            projection: shot
                .events
                .iter()
                .find_map(|e| match e {
                    ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => {
                        Some(p.as_ref().clone())
                    },
                    _ => None,
                })
                .unwrap(),
        }
        .validate_before(Some(&last_checkpoint))
        .unwrap();
        assert!(shot
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::AttackStarted { .. })));
        let held = &last.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(!held.paperdoll.contains_key(&EquipSlot::Lhand));
        assert!(!held.inventory.contains_key(&arrow));
        ZoneState::from_snapshot_with_items(last.snapshot(), Some(Arc::clone(&rules))).unwrap();
        // Accepted start pays MP once and one arrow, and freezes bow timing.
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        zone.enable_items_replay_evidence().unwrap();
        let mp = zone.entities[&owner.entity].combat.as_ref().unwrap().mp;
        let before_checkpoint = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, zone.next_tick, &zone.phase3.as_ref().unwrap().canonical)
            .unwrap();
        let draft = attack(&zone, 2);
        let fired = zone.run_tick(draft.clone()).unwrap();
        assert!(fired.dispositions.is_empty(), "{:?}", fired.dispositions);
        assert_eq!(fired, cold.run_tick(draft).unwrap());
        let c = zone.entities[&owner.entity].combat.as_ref().unwrap();
        let swing = c.swing.unwrap();
        assert_eq!(c.mp, mp - 1);
        let speed = (i128::from(c.sheet.attack_speed().raw()) + i128::from(Q) / 2) / i128::from(Q);
        let attack_ms = 500_000 / speed;
        let reuse_ms = 1500 * 333 / speed;
        let ceil = |ms: i128| u64::try_from((ms + 99) / 100).unwrap();
        assert_eq!(swing.impact.0 - swing.start.0, ceil(attack_ms));
        assert_eq!(swing.ready.0 - swing.start.0, ceil(attack_ms + reuse_ms));
        let held = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert_eq!(held.inventory[&arrow].count, stock - 1);
        let debit = fired
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => Some(p),
                _ => None,
            })
            .unwrap()
            .ledger
            .iter()
            .find(|d| d.operation == ItemOperation::Consume)
            .unwrap()
            .clone();
        assert_eq!((debit.instance, debit.count_delta), (arrow, -1));
        assert!(debit.receipt_refs.is_empty());
        assert_eq!(debit.causes[0].family, "arrow_payment");
        // T26: the API settlement gate accepts the receipt-less arrow debit of a real shot
        // (before, it refused item_consume_cause_identity and fenced the zone).
        let projection = fired
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => Some(p.as_ref()),
                _ => None,
            })
            .unwrap()
            .clone();
        crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection,
        }
        .validate_before(Some(&before_checkpoint))
        .unwrap();
        // L2Character.java:1033 falloff: trunc(D×(distance/4000 + 4/5)), distance in L2 units.
        let mut probe = zone.clone();
        for (offset, damage, expected) in [
            (0, 100, 80),
            (125_000, 100, 180),
            (15_625, 100, 92),
            (8_000, 37, 31),
            (0, 1, 0),
        ] {
            let at = Vec2Fixed::new(Fixed::from_raw(start.x.raw() + offset), start.y);
            let before = probe.entities[&npc.entity].pos;
            probe.entities.get_mut(&npc.entity).unwrap().pos = at;
            probe.aoi.relocate(npc.entity, before, at);
            assert_eq!(
                probe
                    .bow_distance_v6(owner.entity, npc.entity, damage)
                    .unwrap(),
                expected,
                "{offset} {damage}"
            );
        }
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One shielded victim exercised through every auto branch.
    fn v6_auto_attack_shield_block_position_and_draw_order() {
        use crate::domain::zone::{phase4::ShieldOutcome, AttackOutcome, EquipSlot};
        use DamageRollSite as R;
        let (mut zone, owner, npc) = equipment_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let shield_id = crate::domain::zone::ItemInstanceId::new(901).unwrap();
        let player = zone
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
        zone.rebuild_v5(owner.entity).unwrap();
        let command = item_intent(
            &zone,
            owner.entity,
            982,
            crate::domain::zone::ItemCommandKind::Equip {
                instance: shield_id,
                preferred_slot: Some(EquipSlot::Lhand),
            },
        );
        let tick = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::Item(Box::new(command)),
            )]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let radius = zone.effective(npc.entity).unwrap().physical.random_damage();
        let spread = (R::Spread, radius * 2 + 1, radius);
        // Coincident attacker counts as front and inside the shield angle (design §5.3).
        let before = zone.entities[&npc.entity].pos;
        let pos = zone.entities[&owner.entity].pos;
        zone.entities.get_mut(&npc.entity).unwrap().pos = pos;
        zone.aoi.relocate(npc.entity, before, pos);
        let run = |zone: &ZoneState, rolls: &[(DamageRollSite, u32, u32)]| {
            let mut zone = zone.clone();
            let mut trace = Vec::new();
            let result = zone
                .auto_damage_v5(npc, owner, &mut scripted(rolls, &mut trace))
                .unwrap();
            assert_eq!(trace.len(), rolls.len());
            result
        };
        // L2Character.java:1324-1335: hit, shield, critical; a perfect block returns 1
        // before the formula's spread draw (Formulas.java:676-678).
        let perfect = run(
            &zone,
            &[
                (R::AutoHit, 1000, 0),
                (R::ShieldPerfect, 100, 91),
                (R::AutoCritical, 1000, 999),
            ],
        );
        assert_eq!(perfect, (AttackOutcome::Hit, 1));
        let blocked = run(
            &zone,
            &[
                (R::AutoHit, 1000, 0),
                (R::ShieldPerfect, 100, 90),
                (R::ShieldNormal, 100, 0),
                (R::AutoCritical, 1000, 999),
                spread,
            ],
        );
        let open = run(
            &zone,
            &[
                (R::AutoHit, 1000, 0),
                (R::ShieldPerfect, 100, 90),
                (R::ShieldNormal, 100, 99),
                (R::AutoCritical, 1000, 999),
                spread,
            ],
        );
        assert_eq!(blocked.0, AttackOutcome::Hit);
        assert!(blocked.1 < open.1, "{blocked:?} {open:?}");
        // A miss draws nothing after the hit roll.
        assert_eq!(run(&zone, &[(R::AutoHit, 1000, 999)]), (AttackOutcome::Miss, 0));
        // Behind the victim's east facing: no shield draws, back damage multiplier 6/5.
        let mut behind = zone.clone();
        let back = Vec2Fixed::from_tiles(9, 10);
        behind.entities.get_mut(&npc.entity).unwrap().pos = back;
        behind.aoi.relocate(npc.entity, pos, back);
        assert_eq!(
            behind.proximity_v6(npc.entity, owner.entity).unwrap(),
            crate::domain::zone::item_combat::Proximity::Back
        );
        let mut trace = Vec::new();
        assert_eq!(
            behind
                .clone()
                .shield_v6(npc, owner, &mut scripted(&[], &mut trace))
                .unwrap(),
            ShieldOutcome::None
        );
        let rear = run(&behind, &[(R::AutoHit, 1000, 0), (R::AutoCritical, 1000, 999), spread]);
        assert!(rear.1 > open.1, "{rear:?} {open:?}");
        // T51/D157: the defender steps toward the attacker behind it (west), so its facing
        // turns west and the same attacker is now in front: Formulas.calcShldUse rolls the
        // perfect check first (a perfect block returns 1), then the rate check.
        let mut turned = behind.clone();
        let here = turned.entities[&owner.entity].pos;
        turned.entities.get_mut(&owner.entity).unwrap().dest =
            Some(Vec2Fixed::new(Fixed::from_raw(here.x.raw() - 200), here.y));
        turned.step(turned.next_tick);
        turned.entities.get_mut(&owner.entity).unwrap().dest = None;
        assert_eq!(
            turned.phase4.as_ref().unwrap().canonical.facing[&owner.entity].components(),
            (-1, 0)
        );
        assert_eq!(
            run(
                &turned,
                &[
                    (R::AutoHit, 1000, 0),
                    (R::ShieldPerfect, 100, 91),
                    (R::AutoCritical, 1000, 999),
                ],
            ),
            (AttackOutcome::Hit, 1)
        );
        let normal = run(
            &turned,
            &[
                (R::AutoHit, 1000, 0),
                (R::ShieldPerfect, 100, 90),
                (R::ShieldNormal, 100, 0),
                (R::AutoCritical, 1000, 999),
                spread,
            ],
        );
        assert_eq!(normal.0, AttackOutcome::Hit);
        // Side (attacker due north of a west-facing defender, 90 degrees off the 120-degree
        // shield cone): no shield draws.
        let mut side = turned.clone();
        let at = turned.entities[&owner.entity].pos;
        let north = Vec2Fixed::new(at.x, Fixed::from_raw(at.y.raw() + 1000));
        let from = side.entities[&npc.entity].pos;
        side.entities.get_mut(&npc.entity).unwrap().pos = north;
        side.aoi.relocate(npc.entity, from, north);
        let mut trace = Vec::new();
        assert_eq!(
            side.clone()
                .shield_v6(npc, owner, &mut scripted(&[], &mut trace))
                .unwrap(),
            ShieldOutcome::None
        );
        // D153 (T44): a V6 player attacker's shield check is the DEFENDER's fact (the shield
        // owner), with the attacker in the context's `attacker_entity`.
        let attacker = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Player && e.id != owner.entity)
            .map(|e| e.id)
            .unwrap();
        let attacker = zone.binding(attacker).unwrap();
        let mut pvp = zone.clone();
        let from = pvp.entities[&attacker.entity].pos;
        pvp.entities.get_mut(&attacker.entity).unwrap().pos = pos;
        pvp.aoi.relocate(attacker.entity, from, pos);
        pvp.enable_items_replay_evidence().unwrap();
        let mut trace = Vec::new();
        assert_eq!(
            pvp.shield_v6(
                attacker,
                owner,
                &mut scripted(&[(R::ShieldPerfect, 100, 91)], &mut trace)
            )
            .unwrap(),
            ShieldOutcome::Perfect
        );
        let facts = pvp.take_items_replay_evidence();
        let shield = facts
            .iter()
            .find_map(|f| match &f.fact {
                crate::domain::zone::evidence::ItemEvidenceData::Formula(v)
                    if v["formula"] == "shield" =>
                {
                    Some((f.entity, v.clone()))
                },
                _ => None,
            })
            .unwrap();
        assert_eq!(shield.0, owner.entity, "filed under the shield owner");
        assert_eq!(shield.1["context"]["attacker_entity"], attacker.entity.to_string());
        assert!(shield.1["context"].get("target_entity").is_none());
        assert_eq!(shield.1["actual"]["outcome"], "perfect");
        // The legacy V5 formula keeps its draw order without shield or position terms.
        let mut legacy = zone.clone();
        legacy.phase4 = None;
        let mut trace = Vec::new();
        let result = legacy
            .auto_damage_v5(
                npc,
                owner,
                &mut scripted(
                    &[(R::AutoHit, 1000, 0), (R::AutoCritical, 1000, 999), spread],
                    &mut trace,
                ),
            )
            .unwrap();
        assert_eq!(result.0, AttackOutcome::Hit);
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
            enchant: 0,
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
        // D97 admits Soulshot: No Grade 1835 itself; a potion is still not a shot.
        for invalid in [1060, 57, u32::MAX] {
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
            // D76: canonical row positions on the one weapon instance.
            vec![0, 1, 2]
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
                    (!load_family(&value)).then_some(value)
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
                    (!load_family(value)).then_some(value)
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
        let before_checkpoint = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, zone.next_tick, &zone.phase3.as_ref().unwrap().canonical)
            .unwrap();
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
        // D133: two soulshot_reduction draws. The toggle enabled 1464 and its own recharge
        // disabled it (not yet automatic at tick start): not net-removed, so no entry.
        assert_eq!(projection.evidence.len(), 2);
        assert!(projection
            .evidence
            .iter()
            .all(|e| e.family != "auto_shot_exhausted"));
        // engine-review-9 P2-1: settle through the API account against the real pre-tick
        // checkpoint, with the bound rules.
        crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection: projection.as_ref().clone(),
        }
        .validate_before_with(Some(&before_checkpoint), Some(&rules))
        .unwrap();
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

    /// The owner's checkpoint before the next tick and the tick's own projection.
    fn shot_tick(
        zone: &mut ZoneState,
        owner: TargetBinding,
        inputs: Vec<ZoneInput>,
    ) -> (
        AppliedTick,
        crate::domain::zone::Phase4Checkpoint,
        crate::domain::zone::Phase4Projection,
    ) {
        let before = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, zone.next_tick, &zone.phase3.as_ref().unwrap().canonical)
            .unwrap();
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let draft = zone.draft(inputs);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        let projection = applied
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => {
                    Some(p.as_ref().clone())
                },
                _ => None,
            })
            .expect("the removal tick publishes");
        (applied, before, projection)
    }

    /// Settle a tick through the API gate's D127/D130 account (D133): the un-aligned
    /// pre-tick checkpoint and the bound rules.
    fn settle(
        zone: &ZoneState,
        before: &crate::domain::zone::Phase4Checkpoint,
        projection: crate::domain::zone::Phase4Projection,
    ) {
        crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection,
        }
        .validate_before_with(Some(before), Some(&zone.phase4.as_ref().unwrap().rules))
        .unwrap();
    }

    /// D142 (T36): re-admitting an owner whose retained checkpoint holds weapon charges and
    /// automatic shots records the D71 reset on the admission tick: one receipt-less Use row
    /// per charged instance (cause `incarnation_reset`), one D79 step, and one
    /// `auto_shot_exhausted` entry (reason `incarnation`) per dropped toggle. A retained
    /// checkpoint with nothing transient publishes no `ItemCheckpoint`.
    #[test]
    #[allow(clippy::too_many_lines)] // Two re-admissions of one owner.
    fn v6_readmission_records_the_incarnation_reset() {
        use crate::domain::zone::phase4_projection::{ItemEvidenceValue as V, ItemOperation};
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, _, ids) = use_fixture(&[1464, 10491, 1463]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let weapon = zone.phase4.as_ref().unwrap().canonical.players[&owner.entity].paperdoll
            [&EquipSlot::Rhand];
        // Equip + toggle-on in one tick: the weapon is charged and 1464 is automatic.
        let item = |zone: &ZoneState, key: u128, seq: u32, kind: K| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                seq,
                ZoneCommand::Item(Box::new(item_intent(zone, owner.entity, key, kind))),
            )
        };
        let inputs = vec![
            item(
                &zone,
                3101,
                1,
                K::Equip {
                    instance: ids[1],
                    preferred_slot: None,
                },
            ),
            item(
                &zone,
                3102,
                2,
                K::ToggleAutoShot {
                    template: 1464,
                    enabled: true,
                },
            ),
        ];
        let (applied, before, projection) = shot_tick(&mut zone, owner, inputs);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        // T37 (c): equip + toggle-on in one tick: one step, settled un-aligned.
        assert_eq!(projection.checkpoint.inventory_revision, before.inventory_revision + 1);
        assert!(projection
            .receipts
            .iter()
            .all(|r| r.result.inventory_revision == projection.checkpoint.inventory_revision));
        settle(&zone, &before, projection);
        let held = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(held.auto_shots.contains(&1464));
        assert!(held.inventory[&weapon].charges != 0, "the weapon is charged");
        // The load the API would retain after that tick (the last ItemCheckpoint).
        let retained_load = |zone: &ZoneState| {
            let phase = &zone.phase3.as_ref().unwrap().canonical;
            let item_checkpoint = zone
                .phase4
                .as_ref()
                .unwrap()
                .canonical
                .checkpoint(owner.entity, zone.next_tick, phase)
                .unwrap();
            let combat = zone.entities[&owner.entity].combat.as_ref().unwrap();
            let CombatRole::Player {
                progression: Some(progression),
                class,
                xp,
            } = &combat.role
            else {
                unreachable!()
            };
            (
                PlayerLoad {
                    phase5: None,
                    phase4: Some(crate::domain::zone::Phase4PlayerLoad {
                        checkpoint: Some(item_checkpoint.clone()),
                        pending: Vec::new(),
                    }),
                    phase3: Some(phase.checkpoint(owner.entity, zone.next_tick).unwrap()),
                    progression: Some(progression.clone()),
                    checkpoint_revision: Some(1),
                    class: class.clone(),
                    xp: *xp,
                    level: combat.sheet.level(),
                    hp: Some(combat.hp),
                    mp: Some(combat.mp),
                    alive: true,
                },
                item_checkpoint,
            )
        };
        let readmit = |zone: &mut ZoneState, load: PlayerLoad, generation: u64| {
            let spawn = ZoneCommand::SpawnPlayer {
                entity: owner.entity,
                name: "Second".into(),
                pos: zone.entities[&owner.entity].pos,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(generation),
                load: Some(Box::new(load)),
            };
            zone.run_tick(zone.draft(vec![
                ZoneInput::system(ZoneCommand::Despawn {
                    entity: owner.entity,
                }),
                ZoneInput::system(spawn),
            ]))
            .unwrap()
        };
        let (load, retained) = retained_load(&zone);
        let mut cold = zone.clone();
        let admitted = readmit(&mut zone, load.clone(), 2);
        assert_eq!(admitted, readmit(&mut cold, load, 2), "deterministic");
        assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
        let projections: Vec<_> = admitted
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => Some(p.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(projections.len(), 1);
        let projection = projections[0].clone();
        assert_eq!(projection.checkpoint.inventory_revision, retained.inventory_revision + 1);
        assert!(projection.checkpoint.auto_shots.is_empty());
        assert!(projection
            .checkpoint
            .instances
            .iter()
            .all(|i| i.charges == 0));
        assert!(projection.receipts.is_empty());
        assert_eq!(projection.ledger.len(), 1);
        let row = &projection.ledger[0];
        assert_eq!((row.operation, row.instance, row.count_delta), (ItemOperation::Use, weapon, 0));
        assert!(row.before.as_ref().unwrap().charges != 0);
        assert_eq!(row.after.as_ref().unwrap().charges, 0);
        assert_eq!(row.causes[0].family, "incarnation_reset");
        let exhausted: Vec<_> = projection
            .evidence
            .iter()
            .filter(|e| e.family == "auto_shot_exhausted")
            .collect();
        assert_eq!(exhausted.len(), 1);
        assert_eq!(exhausted[0].fields["template"], V::Unsigned(1464));
        assert_eq!(exhausted[0].fields["reason"], V::Text("incarnation".into()));
        // Everything but the toggle removal settles today: the charges row, the D79 step.
        let mut without_toggles = projection.clone();
        without_toggles
            .evidence
            .retain(|e| e.family != "auto_shot_exhausted");
        let mut untoggled = retained.clone();
        untoggled.auto_shots.clear();
        crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection: without_toggles,
        }
        .validate_before_with(Some(&untoggled), Some(&rules))
        .unwrap();
        let settled = crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection,
        }
        .validate_before_with(Some(&retained), Some(&rules));
        // D142: the API account (session 25, merged) accepts the reason-`incarnation` entries.
        settled.unwrap();
        // (b) nothing transient retained: no ItemCheckpoint on re-admission.
        let (load, _) = retained_load(&zone);
        let again = readmit(&mut zone, load, 3);
        assert!(again.dispositions.is_empty(), "{:?}", again.dispositions);
        assert!(!again
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity)));
        // T37 on the admission tick: an owner absent at tick start is admitted, then a
        // toggle-on that changes only auto_shots (1463 is D grade: no charge, no row) and an
        // equip run in that same tick. One step from the admitted revision; receipts agree.
        let (load, retained) = retained_load(&zone);
        let session = |key: u128, seq: u32, kind: K| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(4),
                seq,
                ZoneCommand::Item(Box::new(item_intent(&zone, owner.entity, key, kind))),
            )
        };
        let toggle = session(
            3103,
            1,
            K::ToggleAutoShot {
                template: 1463,
                enabled: true,
            },
        );
        let unequip = session(
            3104,
            2,
            K::Unequip {
                slot: EquipSlot::Under,
            },
        );
        zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn {
            entity: owner.entity,
        })]))
        .unwrap();
        let spawn = ZoneCommand::SpawnPlayer {
            entity: owner.entity,
            name: "Second".into(),
            pos: Vec2Fixed::from_tiles(20, 20),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(4),
            load: Some(Box::new(load)),
        };
        let admitted = zone
            .run_tick(zone.draft(vec![ZoneInput::system(spawn), toggle, unequip]))
            .unwrap();
        assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
        let projection = admitted
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => {
                    Some(p.as_ref().clone())
                },
                _ => None,
            })
            .unwrap();
        let revision = projection.checkpoint.inventory_revision;
        assert_eq!(revision, retained.inventory_revision + 1);
        assert_eq!(projection.receipts.len(), 2);
        assert!(projection
            .receipts
            .iter()
            .all(|r| r.result.inventory_revision == revision));
        settle(&zone, &retained, projection);
    }

    /// T37 (D79): every ordered pair of same-tick item commands steps the owner once, every
    /// receipt carries the settled revision, and the tick settles through the API account
    /// against the real pre-tick checkpoint (equip + toggle-on included).
    #[test]
    #[allow(clippy::too_many_lines)] // One fixture, every ordered command pair.
    fn v6_same_tick_command_pairs_carry_one_revision() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut base, owner, _, ids) = use_fixture(&[1464, 1463, 10491, 4814]);
        base.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .get_mut(&ids[0])
            .unwrap()
            .count = 40;
        base.rebuild_v5(owner.entity).unwrap();
        let rules = Arc::clone(&base.phase4.as_ref().unwrap().rules);
        let account = match &base.entities[&owner.entity].combat.as_ref().unwrap().role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => p.identity.account_id,
            _ => unreachable!(),
        };
        // Mutating commands only: a no-op receipt may keep the pre-tick revision (D143).
        let names = ["equip", "weapon", "unequip", "toggle", "destroy", "grant"];
        let input = |zone: &ZoneState, name: &str, key: u128, seq: u32| -> ZoneInput {
            let kind = match name {
                "equip" => K::Equip {
                    instance: ids[2],
                    preferred_slot: None,
                },
                "weapon" => K::Equip {
                    instance: ids[3],
                    preferred_slot: Some(EquipSlot::Rhand),
                },
                "unequip" => K::Unequip {
                    slot: EquipSlot::Head,
                },
                "toggle" => K::ToggleAutoShot {
                    template: 1464,
                    enabled: true,
                },
                "destroy" => K::Destroy {
                    instance: ids[1],
                    count: 1,
                },
                _ => {
                    let grant = crate::domain::zone::item_grant::ItemGrantCommand {
                        entity: owner.entity,
                        account,
                        request_id: Uuid::from_u128(key),
                        template_id: 1463,
                        count: 2,
                        fingerprint: crate::domain::zone::item_grant::grant_fingerprint(
                            crate::domain::CharacterId::from_uuid(owner.entity.as_uuid()),
                            1463,
                            2,
                        ),
                        rules_hash: rules.hash(),
                        paused: None,
                        known: None,
                        conflict: None,
                        enchant: 0,
                    };
                    return ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(
                        grant,
                    )));
                },
            };
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                seq,
                ZoneCommand::Item(Box::new(item_intent(zone, owner.entity, key, kind))),
            )
        };
        let mut key = 3000_u128;
        for first in names {
            for second in names {
                if first == second {
                    continue;
                }
                let mut zone = base.clone();
                key += 2;
                let inputs = vec![
                    input(&zone, first, key, 1),
                    input(&zone, second, key + 1, 2),
                ];
                let (applied, before, projection) = shot_tick(&mut zone, owner, inputs);
                assert!(
                    applied.dispositions.is_empty(),
                    "{first}+{second}: {:?}",
                    applied.dispositions
                );
                let revision = projection.checkpoint.inventory_revision;
                assert_eq!(revision, before.inventory_revision + 1, "{first}+{second}");
                for receipt in &projection.receipts {
                    assert_eq!(
                        receipt.result.inventory_revision, revision,
                        "{first}+{second}: {receipt:?}"
                    );
                }
                for event in &applied.events {
                    if let ZoneEvent::ItemResult(result) = event {
                        assert_eq!(result.inventory_revision, revision, "{first}+{second}");
                    }
                }
                crate::application::ports::ItemCheckpointWork {
                    zone: zone.seed.zone,
                    epoch: zone.seed.epoch,
                    allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
                    projection,
                }
                .validate_before_with(Some(&before), Some(&rules))
                .unwrap_or_else(|e| panic!("{first}+{second}: {e:?}"));
            }
        }
        // P3-2: an owner admitted THIS tick with a retained toggle (an entry-only D142 reset),
        // then equip + toggle-on in the same tick: one step from the retained revision.
        let mut zone = base.clone();
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .auto_shots
            .insert(1463);
        let phase = &zone.phase3.as_ref().unwrap().canonical;
        let retained = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, zone.next_tick, phase)
            .unwrap();
        assert!(retained.instances.iter().all(|item| item.charges == 0));
        let combat = zone.entities[&owner.entity].combat.as_ref().unwrap();
        let CombatRole::Player {
            progression: Some(progression),
            class,
            xp,
        } = &combat.role
        else {
            unreachable!()
        };
        let load = PlayerLoad {
            phase5: None,
            phase4: Some(crate::domain::zone::Phase4PlayerLoad {
                checkpoint: Some(retained.clone()),
                pending: Vec::new(),
            }),
            phase3: Some(phase.checkpoint(owner.entity, zone.next_tick).unwrap()),
            progression: Some(progression.clone()),
            checkpoint_revision: Some(1),
            class: class.clone(),
            xp: *xp,
            level: combat.sheet.level(),
            hp: Some(combat.hp),
            mp: Some(combat.mp),
            alive: true,
        };
        let session = |zone: &ZoneState, key: u128, seq: u32, kind: K| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(2),
                seq,
                ZoneCommand::Item(Box::new(item_intent(zone, owner.entity, key, kind))),
            )
        };
        let equip = session(
            &zone,
            3301,
            1,
            K::Equip {
                instance: ids[2],
                preferred_slot: None,
            },
        );
        let toggle = session(
            &zone,
            3302,
            2,
            K::ToggleAutoShot {
                template: 1464,
                enabled: true,
            },
        );
        zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn {
            entity: owner.entity,
        })]))
        .unwrap();
        let spawn = ZoneCommand::SpawnPlayer {
            entity: owner.entity,
            name: "Second".into(),
            pos: Vec2Fixed::from_tiles(20, 20),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(2),
            load: Some(Box::new(load)),
        };
        let admitted = zone
            .run_tick(zone.draft(vec![ZoneInput::system(spawn), equip, toggle]))
            .unwrap();
        assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
        let projection = admitted
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => {
                    Some(p.as_ref().clone())
                },
                _ => None,
            })
            .unwrap();
        let revision = projection.checkpoint.inventory_revision;
        assert_eq!(revision, retained.inventory_revision + 1);
        assert_eq!(projection.receipts.len(), 2);
        assert!(projection
            .receipts
            .iter()
            .all(|r| r.result.inventory_revision == revision));
        assert!(projection
            .evidence
            .iter()
            .any(|e| e.family == "auto_shot_exhausted"
                && e.fields.get("reason")
                    == Some(&crate::domain::zone::phase4_projection::ItemEvidenceValue::Text(
                        "incarnation".into()
                    ))));
        let settled = crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection,
        }
        .validate_before_with(Some(&retained), Some(&rules));
        // T38/D146 (API session 27): the account judges the `incarnation` entries and reset rows
        // against the admission reset itself, so a same-tick toggle-on that re-enables and
        // charges after admission settles under its own receipt.
        assert!(settled.is_ok(), "{settled:?}");
    }

    /// D133 (engine-review-8 P1-1/P2-1): entries are normalized against the tick-start set.
    /// (a) Toggle-enable of a not-yet-automatic template with too few shots: the toggle adds
    /// it and its own recharge disables it, so it is not net-removed and has no entry.
    /// (b) In one tick, a toggle removes an automatic template whose stack is gone (entry), a
    /// grant adds shots, and a toggle re-enables it: unchanged over the tick, so no entry.
    /// Both ticks settle un-aligned through the API's D127/D130 account.
    #[test]
    #[allow(clippy::too_many_lines)] // Two one-tick scenarios over one fixture.
    fn v6_auto_shot_entries_follow_the_tick_net_change() {
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let (mut zone, owner, _, ids) = use_fixture(&[4814, 1464]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let required = rules.parts().items[&4814].soulshots;
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        let light = player.paperdoll[&EquipSlot::Rhand];
        let light_required = rules.parts().items[&player.inventory[&light].template_id].soulshots;
        assert!(light_required < required, "{light_required} {required}");
        player.inventory.get_mut(&ids[0]).unwrap().count = 1;
        player.inventory.get_mut(&ids[1]).unwrap().count = u64::from(required) - 1;
        assert!(u64::from(required) > u64::from(light_required));
        let item = |zone: &ZoneState, key: u128, seq: u32, kind: K| {
            ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                seq,
                ZoneCommand::Item(Box::new(item_intent(zone, owner.entity, key, kind))),
            )
        };
        let equip = item(
            &zone,
            2600,
            1,
            K::Equip {
                instance: ids[0],
                preferred_slot: Some(EquipSlot::Rhand),
            },
        );
        let equipped = zone.run_tick(zone.draft(vec![equip])).unwrap();
        assert!(equipped.dispositions.is_empty(), "{:?}", equipped.dispositions);
        let entries = |projection: &crate::domain::zone::Phase4Projection| {
            projection
                .evidence
                .iter()
                .filter(|e| e.family == "auto_shot_exhausted")
                .count()
        };
        // (a) toggle-enable, not yet automatic, too few shots.
        let mut toggle = zone.clone();
        let enable = item(
            &toggle,
            2601,
            2,
            K::ToggleAutoShot {
                template: 1464,
                enabled: true,
            },
        );
        let (_, before, projection) = shot_tick(&mut toggle, owner, vec![enable]);
        assert!(!before.auto_shots.contains(&1464));
        assert!(!projection.checkpoint.auto_shots.contains(&1464));
        assert_eq!(entries(&projection), 0);
        settle(&toggle, &before, projection);
        // (b) 1464 is automatic at tick start with no stack. A toggle of 1463 visits it and
        // removes it (`missing`, an entry at that point); a dev grant then adds 1464 shots and
        // a toggle re-enables 1464, all in the command phase of one tick. Staged Use and the
        // attack/cast recharges run after every item command, so this is the only in-tick
        // remove-then-re-enable order.
        let (mut zone, owner, _, ids) = use_fixture(&[1463, 1464]);
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        player.inventory.remove(&ids[1]);
        player.auto_shots.insert(1464);
        zone.rebuild_v5(owner.entity).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let account = match &zone.entities[&owner.entity].combat.as_ref().unwrap().role {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => p.identity.account_id,
            _ => unreachable!(),
        };
        let grant = crate::domain::zone::item_grant::ItemGrantCommand {
            entity: owner.entity,
            account,
            request_id: Uuid::from_u128(2605),
            template_id: 1464,
            count: 5,
            fingerprint: crate::domain::zone::item_grant::grant_fingerprint(
                crate::domain::CharacterId::from_uuid(owner.entity.as_uuid()),
                1464,
                5,
            ),
            rules_hash: rules.hash(),
            paused: None,
            known: None,
            conflict: None,
            enchant: 0,
        };
        let inputs = vec![
            item(
                &zone,
                2606,
                1,
                K::ToggleAutoShot {
                    template: 1463,
                    enabled: true,
                },
            ),
            ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(grant))),
            item(
                &zone,
                2607,
                2,
                K::ToggleAutoShot {
                    template: 1464,
                    enabled: true,
                },
            ),
        ];
        let (applied, before, projection) = shot_tick(&mut zone, owner, inputs);
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert!(before.auto_shots.contains(&1464));
        assert!(projection.checkpoint.auto_shots.contains(&1464), "re-enabled");
        assert!(projection.checkpoint.auto_shots.contains(&1463));
        assert_eq!(entries(&projection), 0, "{:?}", projection.evidence);
        settle(&zone, &before, projection);
    }

    /// engine-review-7 P1-1 / D128: a manual Use with fewer shots than the weapon needs
    /// refuses (`InsufficientItems`) and disables the automatic shot with its entry; the
    /// toggle-enable path's `missing` removal of another template carries one too.
    #[test]
    #[allow(clippy::too_many_lines)] // Two disable paths over one fixture.
    fn v6_every_auto_shot_disable_path_records_its_entry() {
        use crate::domain::zone::phase4_projection::ItemEvidenceValue as V;
        use crate::domain::zone::{EquipSlot, ItemCommandKind as K};
        let entries = |projection: &crate::domain::zone::Phase4Projection| {
            projection
                .evidence
                .iter()
                .filter(|e| e.family == "auto_shot_exhausted")
                .cloned()
                .collect::<Vec<_>>()
        };
        // Weapon 4814 with soulshot 1464 automatic and fewer shots than one charge.
        let (mut zone, owner, _, ids) = use_fixture(&[4814, 1464]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let required = rules.parts().items[&4814].soulshots;
        assert!(required > 1, "{required}");
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        player.inventory.get_mut(&ids[0]).unwrap().count = 1;
        player.inventory.get_mut(&ids[1]).unwrap().count = u64::from(required) - 1;
        let equip = zone
            .run_tick(zone.draft(vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                ZoneCommand::Item(Box::new(item_intent(
                    &zone,
                    owner.entity,
                    2500,
                    K::Equip {
                        instance: ids[0],
                        preferred_slot: Some(EquipSlot::Rhand),
                    },
                ))),
            )]))
            .unwrap();
        assert!(equip.dispositions.is_empty(), "{:?}", equip.dispositions);
        let mut toggle = zone.clone();
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .auto_shots
            .insert(1464);
        let manual = use_input(&zone, owner, 2501, 2, ids[1], None);
        let (applied, before, projection) = shot_tick(&mut zone, owner, vec![manual]);
        assert_eq!(applied.dispositions.len(), 1, "{:?}", applied.dispositions);
        assert_eq!(applied.dispositions[0].reason, RejectReason::InsufficientItems);
        assert!(projection.receipts.is_empty());
        assert!(!projection.checkpoint.auto_shots.contains(&1464));
        let found = entries(&projection);
        assert_eq!(found.len(), 1, "{found:?}");
        let entry = &found[0].fields;
        assert_eq!(entry["template"], V::Unsigned(1464));
        assert_eq!(entry["reason"], V::Text("insufficient".into()));
        assert_eq!(entry["instance"], V::Unsigned(ids[1].get()));
        assert_eq!(entry["remaining"], V::Unsigned(u64::from(required) - 1));
        assert_eq!(entry["required"], V::Unsigned(u64::from(required)));
        assert_eq!(entry["weapon"], V::Unsigned(ids[0].get()));
        settle(&zone, &before, projection);
        // Toggle-enable: 1463 is automatic with no stack left; enabling 1464 visits it.
        toggle
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .auto_shots
            .insert(1463);
        let input = ZoneInput::session(
            owner.entity,
            SessionGeneration(1),
            2,
            ZoneCommand::Item(Box::new(item_intent(
                &toggle,
                owner.entity,
                2502,
                K::ToggleAutoShot {
                    template: 1464,
                    enabled: true,
                },
            ))),
        );
        let (_, before, projection) = shot_tick(&mut toggle, owner, vec![input]);
        assert!(!projection.checkpoint.auto_shots.contains(&1463));
        let found = entries(&projection);
        let missing = found
            .iter()
            .find(|e| e.fields["template"] == V::Unsigned(1463))
            .expect("missing entry");
        assert_eq!(missing.fields["reason"], V::Text("missing".into()));
        // D133: 1464 was enabled and disabled within this tick, so it has no entry.
        assert!(found
            .iter()
            .all(|e| e.fields["template"] != V::Unsigned(1464)));
        assert_eq!(missing.fields["remaining"], V::Unsigned(0));
        assert!(!missing.fields.contains_key("instance"));
        assert_eq!(missing.fields["weapon"], V::Unsigned(ids[0].get()));
        assert_eq!(
            missing.fields["required"],
            V::Unsigned(u64::from(required)),
            "1463 is a soulshot too"
        );
        // One entry per removed template, in template order.
        let templates = found
            .iter()
            .map(|e| e.fields["template"].clone())
            .collect::<Vec<_>>();
        let mut sorted = templates.clone();
        sorted.sort_by_key(|v| match v {
            V::Unsigned(n) => *n,
            _ => u64::MAX,
        });
        sorted.dedup();
        assert_eq!(templates, sorted);
        settle(&toggle, &before, projection);
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Actual swing start, impact, discharge/retention and cold parity.
    fn v6_shot_automatic_attack_recharges_at_swing_start_and_discharges_only_on_landing() {
        use crate::domain::zone::phase4::ItemActionBinding;
        use crate::domain::zone::phase4_projection::ItemOperation;
        use crate::domain::zone::{AttackOutcome, EquipSlot};
        // C-grade 1464 matches Flamberge; D-grade 1463 is a persisted mismatch (I2).
        let (mut zone, owner, npc, ids) = use_fixture(&[1464, 1463]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let mut plain = zone.clone();
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        let weapon = player.paperdoll[&EquipSlot::Rhand];
        player.auto_shots.extend([1463, 1464]);
        player.inventory.get_mut(&ids[0]).unwrap().count = 20;
        let weapon_template = player.inventory[&weapon].template_id;
        assert_eq!(rules.parts().items[&weapon_template].soulshots, 2);
        // Flamberge has no reduced-soulshot table, so recharge draws nothing: the plain
        // run without toggles consumes the identical RNG stream.
        assert!(rules.parts().items[&weapon_template]
            .reduced_soulshot
            .is_empty());
        plain
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap()
            .inventory
            .get_mut(&ids[0])
            .unwrap()
            .count = 20;
        zone.rebuild_v5(owner.entity).unwrap();
        plain.rebuild_v5(owner.entity).unwrap();
        let mut cold =
            match ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))) {
                Ok(cold) => cold,
                Err(error) => panic!("{error:?}"),
            };
        zone.enable_items_replay_evidence().unwrap();
        let start = vec![
            review_input(
                owner.entity,
                1,
                ZoneCommand::SetTarget {
                    entity: owner.entity,
                    target: Some(npc.entity),
                },
            ),
            review_input(
                owner.entity,
                2,
                ZoneCommand::Attack {
                    entity: owner.entity,
                },
            ),
        ];
        let mut drafts = Vec::new();
        let mut results = Vec::new();
        let mut expect_payment = true;
        let mut stock = 20_u64;
        let (mut hits, mut misses, mut doubled) = (0, 0, 0);
        for step in 0..400 {
            let draft = zone.draft(if step == 0 { start.clone() } else { Vec::new() });
            let hp_before = zone.entities[&npc.entity].combat.as_ref().unwrap().hp;
            let applied = zone.run_tick(draft.clone()).unwrap();
            let baseline = plain.run_tick(draft.clone()).unwrap();
            assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
            let owner_rows = |kind: ItemOperation, instance| {
                applied
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => Some(p),
                        _ => None,
                    })
                    .flat_map(|p| p.ledger.iter())
                    .filter(|d| d.operation == kind && d.instance == instance)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let result = |events: &[ZoneEvent]| {
                events.iter().find_map(|e| match e {
                    ZoneEvent::AttackResult {
                        attacker,
                        outcome,
                        damage,
                        target_hp_after,
                        ..
                    } if *attacker == owner.entity => Some((*outcome, *damage, *target_hp_after)),
                    _ => None,
                })
            };
            let mut target_died = false;
            if let Some((outcome, damage, hp_after)) = result(&applied.events) {
                let (plain_outcome, plain_damage, plain_hp) = result(&baseline.events).unwrap();
                assert_eq!(outcome, plain_outcome);
                let discharges = owner_rows(ItemOperation::Use, weapon);
                if outcome == AttackOutcome::Miss {
                    // All-miss retains the paid charge for the next bundle.
                    assert!(discharges.is_empty());
                    misses += 1;
                    expect_payment = false;
                } else {
                    assert_eq!(discharges.len(), 1);
                    assert_eq!(discharges[0].causes[0].family, "shot_discharge");
                    assert_eq!(discharges[0].after.as_ref().unwrap().charges & 1, 0);
                    // A lethal hit is capped by the remaining HP in both runs.
                    assert!(damage >= plain_damage && damage <= hp_before);
                    hits += 1;
                    expect_payment = true;
                }
                target_died = hp_after == 0 || plain_hp == 0;
            }
            if applied.events.iter().any(|e| {
                matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == owner.entity)
            }) {
                if doubled == 0 {
                    // Formulas.java:684,690: the frozen soulshot doubles the non-critical
                    // P.Atk term before truncation; the identical forced draws without the
                    // bundle's binding give the plain value.
                    let mut charged = zone.clone();
                    let mut uncharged = zone.clone();
                    let items = &mut uncharged.phase4.as_mut().unwrap().canonical;
                    items.attack_plans.clear();
                    items.charge_bindings.clear();
                    let source = zone.binding(owner.entity).unwrap();
                    let target = zone.binding(npc.entity).unwrap();
                    let mut forced = |z: &mut ZoneState, site, _, bound| {
                        let value = z.roll_below(bound);
                        Ok(match site {
                            DamageRollSite::AutoHit => 0,
                            DamageRollSite::AutoCritical => bound - 1,
                            _ => value,
                        })
                    };
                    let (ss_outcome, ss) =
                        charged.auto_damage_v5(source, target, &mut forced).unwrap();
                    let (outcome, base) =
                        uncharged.auto_damage_v5(source, target, &mut forced).unwrap();
                    assert_eq!((ss_outcome, outcome), (AttackOutcome::Hit, AttackOutcome::Hit));
                    assert!(base > 1 && (ss == 2 * base || ss == 2 * base + 1), "{ss}/{base}");
                    assert_eq!(charged.rng.get_word_pos(), uncharged.rng.get_word_pos());
                    doubled += 1;
                }
                let items = &zone.phase4.as_ref().unwrap().canonical;
                let bundle = &items.attack_plans[&owner.entity];
                assert_eq!((bundle.shots, bundle.weapon), (1, Some(weapon)));
                assert_eq!(
                    items.charge_bindings,
                    vec![crate::domain::zone::phase4::ChargeBinding {
                        weapon,
                        shot: crate::domain::zone::phase4::ShotKind::Soul,
                        action: ItemActionBinding::Attack {
                            attacker: owner.entity,
                            start: bundle.start
                        }
                    }]
                );
                let debits = owner_rows(ItemOperation::Consume, ids[0]);
                if expect_payment {
                    assert_eq!(debits.len(), 1);
                    assert_eq!(debits[0].count_delta, -2);
                    assert!(debits[0].receipt_refs.is_empty());
                    assert_eq!(debits[0].causes[0].family, "auto_shot_payment");
                    stock -= 2;
                } else {
                    assert!(debits.is_empty());
                }
                let held = &items.players[&owner.entity];
                assert_eq!(held.inventory[&ids[0]].count, stock);
                assert_eq!(held.inventory[&ids[1]].count, 3);
                assert!(held.auto_shots.contains(&1463));
            }
            drafts.push(draft);
            results.push(applied);
            if target_died || (hits >= 2 && misses >= 1) || hits >= 6 {
                break;
            }
        }
        // The fixed fixture stream yields both branches: retained misses and a landed hit.
        assert!(hits >= 1 && misses >= 1 && doubled == 1, "{hits}/{misses}/{doubled}");
        for (draft, expected) in drafts.into_iter().zip(results) {
            assert_eq!(cold.run_tick(draft).unwrap(), expected);
        }
        assert_eq!(zone.snapshot(), cold.snapshot());
        ZoneState::from_snapshot_with_items(zone.snapshot(), Some(rules)).unwrap();
    }

    /// T28/D127: an attack-start recharge that only removes an automatic shot (no stack left
    /// at the attempt, or fewer than one charge) still publishes the owner's `ItemCheckpoint`:
    /// a changed tick (D79 revision step) carrying its `auto_shot_exhausted` evidence.
    #[test]
    fn v6_attack_auto_shot_only_removal_publishes_its_checkpoint() {
        use crate::domain::zone::phase4_projection::ItemEvidenceValue as V;
        use crate::domain::zone::EquipSlot;
        // Flamberge (C) needs 2 soulshots per charge; 1464 is its C-grade soulshot.
        for (stock, reason) in [(None, "missing"), (Some(1_u64), "insufficient")] {
            let (mut zone, owner, npc, ids) = use_fixture(&[1464]);
            let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
            let player = zone
                .phase4
                .as_mut()
                .unwrap()
                .canonical
                .players
                .get_mut(&owner.entity)
                .unwrap();
            let weapon = player.inventory[&player.paperdoll[&EquipSlot::Rhand]].template_id;
            assert_eq!(rules.parts().items[&weapon].soulshots, 2);
            player.auto_shots.insert(1464);
            match stock {
                Some(count) => player.inventory.get_mut(&ids[0]).unwrap().count = count,
                None => {
                    player.inventory.remove(&ids[0]);
                },
            }
            zone.rebuild_v5(owner.entity).unwrap();
            zone.run_tick(zone.draft(Vec::new())).unwrap();
            let mut cold =
                ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules)))
                    .unwrap();
            let start = vec![
                review_input(
                    owner.entity,
                    1,
                    ZoneCommand::SetTarget {
                        entity: owner.entity,
                        target: Some(npc.entity),
                    },
                ),
                review_input(
                    owner.entity,
                    2,
                    ZoneCommand::Attack {
                        entity: owner.entity,
                    },
                ),
            ];
            let mut published = None;
            for step in 0..200 {
                let before = zone
                    .phase4
                    .as_ref()
                    .unwrap()
                    .canonical
                    .checkpoint(
                        owner.entity,
                        zone.next_tick,
                        &zone.phase3.as_ref().unwrap().canonical,
                    )
                    .unwrap();
                let draft = zone.draft(if step == 0 { start.clone() } else { Vec::new() });
                let applied = zone.run_tick(draft.clone()).unwrap();
                assert_eq!(applied, cold.run_tick(draft).unwrap());
                assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
                if let Some(projection) = applied.events.iter().find_map(|e| match e {
                    ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => {
                        Some(p.as_ref().clone())
                    },
                    _ => None,
                }) {
                    published = Some((before, projection));
                    break;
                }
                assert!(zone.phase4.as_ref().unwrap().canonical.players[&owner.entity]
                    .auto_shots
                    .contains(&1464));
            }
            let (before, projection) = published.expect("the removal tick publishes");
            assert!(before.auto_shots.contains(&1464));
            assert!(!projection.checkpoint.auto_shots.contains(&1464));
            assert!(projection.ledger.is_empty(), "{:?}", projection.ledger);
            assert_eq!(
                projection.checkpoint.inventory_revision,
                before.inventory_revision.checked_add(1).unwrap(),
                "a changed tick strictly advances (D79)"
            );
            let evidence = projection
                .evidence
                .iter()
                .filter(|e| e.family == "auto_shot_exhausted")
                .collect::<Vec<_>>();
            assert_eq!(evidence.len(), 1, "{evidence:?}");
            assert_eq!(evidence[0].fields["template"], V::Unsigned(1464));
            assert_eq!(evidence[0].fields["reason"], V::Text(reason.into()));
            assert_eq!(evidence[0].fields["remaining"], V::Unsigned(stock.unwrap_or(0)));
            // The API's D127 rule (session 21): removal-only, and the removed template has no
            // remaining instance, or `insufficient` with remaining below the weapon's
            // per-shot requirement.
            let held: u64 = projection
                .checkpoint
                .instances
                .iter()
                .filter(|item| item.template_id == 1464)
                .map(|item| item.count)
                .sum();
            assert!(projection
                .checkpoint
                .auto_shots
                .is_subset(&before.auto_shots));
            assert!(held == 0 || (reason == "insufficient" && held < 2));
            // D133: the whole tick settles un-aligned through the API's D127/D130 account.
            settle(&zone, &before, projection);
        }
    }

    /// D127: the automatic charge that consumes the last shots of a stack disables that
    /// automatic shot in the same tick; its Consume row (`auto_shot_payment`) and the
    /// `auto_shot_exhausted` evidence reference the emptied stack with one resolution id.
    #[test]
    fn v6_automatic_shot_exhausting_its_stack_is_removed_with_evidence() {
        use crate::domain::zone::phase4_projection::{ItemEvidenceValue as V, ItemOperation};
        let (mut zone, owner, npc, ids) = use_fixture(&[2511]);
        let rules = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        player.auto_shots.insert(2511);
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
        // Probe the per-charge count, then leave exactly one charge in the stack.
        let mut probe = zone.clone();
        let input = review_input(
            owner.entity,
            1,
            ZoneCommand::CastSkill {
                entity: owner.entity,
                skill_id: crate::domain::zone::skill::SkillId(1177),
                target: Some(npc.entity),
            },
        );
        probe.run_tick(probe.draft(vec![input.clone()])).unwrap();
        let per_charge = 3 - probe.phase4.as_ref().unwrap().canonical.players[&owner.entity]
            .inventory[&ids[0]]
            .count;
        assert!(per_charge > 0 && per_charge < 3);
        let player = zone
            .phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .get_mut(&owner.entity)
            .unwrap();
        player.inventory.get_mut(&ids[0]).unwrap().count = per_charge;
        zone.rebuild_v5(owner.entity).unwrap();
        let mut cold =
            ZoneState::from_snapshot_with_items(zone.snapshot(), Some(Arc::clone(&rules))).unwrap();
        let settled_before = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, zone.next_tick, &zone.phase3.as_ref().unwrap().canonical)
            .unwrap();
        let draft = zone.draft(vec![input]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
        assert_eq!(applied, cold.run_tick(draft).unwrap());
        let player = &zone.phase4.as_ref().unwrap().canonical.players[&owner.entity];
        assert!(!player.inventory.contains_key(&ids[0]), "the last shots were paid");
        assert!(!player.auto_shots.contains(&2511), "removed in the same tick");
        let projection = applied
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == owner.entity => Some(p),
                _ => None,
            })
            .unwrap();
        assert!(!projection.checkpoint.auto_shots.contains(&2511));
        let row = projection
            .ledger
            .iter()
            .find(|row| row.instance == ids[0])
            .unwrap();
        assert_eq!(row.operation, ItemOperation::Consume);
        assert!(row.after.is_none());
        assert_eq!(row.causes[0].family, "auto_shot_payment");
        let evidence = projection
            .evidence
            .iter()
            .find(|e| e.family == "auto_shot_exhausted")
            .unwrap();
        assert_eq!(evidence.fields["template"], V::Unsigned(2511));
        assert_eq!(evidence.fields["instance"], V::Unsigned(ids[0].get()));
        assert_eq!(evidence.fields["remaining"], V::Unsigned(0));
        assert_eq!(evidence.fields["reason"], V::Text("exhausted".into()));
        assert_eq!(evidence.fields["resolution_id"], row.causes[0].fields["resolution_id"]);
        // API session 21 (D127/D130): the real tick settles without a toggle receipt against
        // the un-aligned pre-tick checkpoint, with the bound rules.
        crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection: projection.as_ref().clone(),
        }
        .validate_before_with(Some(&settled_before), Some(&rules))
        .unwrap();
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
        let before_checkpoint = zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .checkpoint(owner.entity, zone.next_tick, &zone.phase3.as_ref().unwrap().canonical)
            .unwrap();
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
        // D127: the stack-less 1465 leaves auto_shots with its audit evidence.
        let exhausted = projection
            .evidence
            .iter()
            .filter(|e| e.family == "auto_shot_exhausted")
            .collect::<Vec<_>>();
        assert_eq!(exhausted.len(), 1, "{exhausted:?}");
        assert_eq!(
            exhausted[0].fields["template"],
            crate::domain::zone::phase4_projection::ItemEvidenceValue::Unsigned(1465)
        );
        assert_eq!(
            exhausted[0].fields["reason"],
            crate::domain::zone::phase4_projection::ItemEvidenceValue::Text("missing".into())
        );
        // T26 + D127/D130 (API session 21): the API settlement gate accepts these receipt-less
        // automatic-shot debits and the evidenced `missing` removal against the un-aligned
        // pre-tick checkpoint, with the bound rules.
        crate::application::ports::ItemCheckpointWork {
            zone: zone.seed.zone,
            epoch: zone.seed.epoch,
            allocation: zone.phase4.as_ref().unwrap().canonical.allocator,
            projection: projection.as_ref().clone(),
        }
        .validate_before_with(Some(&before_checkpoint), Some(&zone.phase4.as_ref().unwrap().rules))
        .unwrap();
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
                    (!load_family(value)).then_some(value)
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
        // Formulas.java:760: V6 applies the actual position (owner behind the NPC: ×6/5).
        for (shots, expected) in [(0, 138), (1, 232)] {
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
