//! V5 normal attacks. Selection/timing is shared with legacy; the impact calls the
//! unified recipient and preserves L2Character.java:4080-4152's post-reduction break.
use super::*;
use crate::domain::zone::{AttackOutcome, Swing};

impl ZoneState {
    pub(in super::super) fn land_autos_v5(
        &mut self,
        tick: Tick,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let due: Vec<_> = self
            .entities
            .values()
            .filter_map(|e| {
                let swing = e.combat.as_ref()?.swing?;
                (swing.impact <= tick).then_some((e.id, swing))
            })
            .collect();
        // BTreeMap traversal retains the legacy attacker EntityId order (design phase6).
        for (id, swing) in due {
            let mark = self.evidence_mark();
            let before = if self.evidence.is_some() {
                serde_json::json!({"source": self.evidence_context(id)?,
                    "target": self.entities.get(&swing.target), "swing": swing})
            } else {
                serde_json::Value::Null
            };
            let start = events.len();
            self.land_auto_v5(tick, id, swing, events, &mut draw_damage_v5)?;
            if self.phase4.is_some() {
                let landed = events
                    .get(start..)
                    .ok_or_else(missing)?
                    .iter()
                    .any(|event| {
                        matches!(event, ZoneEvent::AttackResult { attacker, outcome, .. }
                        if *attacker == id && *outcome != AttackOutcome::Miss)
                    });
                let initial = before_items
                    .and_then(|items| items.players.get(&id))
                    .map(|player| player.inventory_revision);
                self.finish_attack_shots_v6(tick, id, swing.start, landed, initial, events)?;
            }
            if self.evidence.is_some() {
                self.evidence_fact(
                    "auto_attack_resolution",
                    id,
                    serde_json::json!({
                        "inputs": before, "events": events.get(start..).ok_or_else(missing)?,
                        "rng_draws": self.evidence_draws(mark)
                    }),
                );
            }
        }
        Ok(())
    }

    pub(super) fn land_auto_v5(
        &mut self,
        tick: Tick,
        id: EntityId,
        swing: Swing,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
        // A previous lethal hit/target change may already have canceled this candidate.
        if self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .and_then(|c| c.swing)
            != Some(swing)
        {
            return Ok(());
        }
        let source = self.binding(id).ok_or_else(missing)?;
        let target = TargetBinding {
            entity: swing.target,
            incarnation: swing.target_incarnation,
        };
        let cancel = if !self.auto_action_v5(id) || self.hostility_v5(tick, source, target).is_err()
        {
            Some(SwingCancel::TargetLost)
        } else {
            let a = self.entities.get(&id).ok_or_else(missing)?;
            let b = self.entities.get(&target.entity).ok_or_else(missing)?;
            let c = a.combat.as_ref().ok_or_else(missing)?;
            let reach = c
                .attack_range
                .saturating_add(b.combat.as_ref().ok_or_else(missing)?.collision_radius);
            if a.targeting.target != Some(target.entity)
                || (a.kind == EntityKind::Player
                    && !self.aoi.in_aoi(a.pos).any(|id| id == target.entity))
            {
                Some(SwingCancel::TargetLost)
            } else if !a.pos.within(b.pos, reach) {
                Some(SwingCancel::OutOfRange)
            } else {
                None
            }
        };
        self.entities
            .get_mut(&id)
            .and_then(|e| e.combat.as_mut())
            .ok_or_else(missing)?
            .swing = None;
        if let Some(reason) = cancel {
            events.push(ZoneEvent::AttackCancelled {
                tick,
                attacker: id,
                target: target.entity,
                reason,
            });
            return Ok(());
        }
        self.note_hostility_v5(tick, source, target)?;
        let (outcome, rolled) = self.auto_damage_v5(source, target, draw)?;
        let damage = if outcome == AttackOutcome::Miss {
            // An evasion is not a reduceCurrentHp call: no status removal/hate/break draw.
            DamageOutcome {
                cp_absorbed: 0,
                hp_removed: 0,
                hp_after: self
                    .entities
                    .get(&target.entity)
                    .and_then(|e| e.combat.as_ref())
                    .ok_or_else(missing)?
                    .hp,
                lethal: false,
            }
        } else {
            let outcome = self.apply_damage_rolled_v5(
                tick,
                DamageRequest {
                    source: self.damage_source_v5(source)?,
                    target,
                    cause: DamageCause::Direct,
                    rolled_damage: rolled,
                },
                events,
                draw,
            )?;
            // Reflection/absorption providers are outside the admitted capability manifest;
            // unlike skill vengeance, normal attacks consume no zero-chance reflection draw.
            // Source auto EVT_ATTACKED follows recipient/reflection, even though attackable
            // reduceCurrentHp has already delivered its own notification before HP removal.
            if self
                .entities
                .get(&target.entity)
                .is_some_and(|e| e.kind == EntityKind::Npc && !e.targeting.dead)
            {
                if let Some(ai) = self
                    .entities
                    .get_mut(&target.entity)
                    .and_then(|e| e.ai.as_mut())
                {
                    ai.last_hit = tick;
                }
                self.add_hate_for(
                    tick,
                    target.entity,
                    source.entity,
                    crate::domain::zone::HateEntry { hate: 1, damage: 0 },
                    "auto_attacked",
                    events,
                );
            }
            self.attack_break_v5(tick, target, rolled, events, draw)?;
            outcome
        };
        events.push(ZoneEvent::AttackResult {
            phase3: Some(crate::domain::zone::AttackResultPhase3 {
                attacker_incarnation: source.incarnation,
                cp_absorbed: damage.cp_absorbed,
            }),
            tick,
            attacker: id,
            target: target.entity,
            target_incarnation: target.incarnation,
            outcome,
            damage: damage.hp_removed,
            target_hp_after: damage.hp_after,
        });
        Ok(())
    }

    /// Formulas.java:1054-1063 and :665-743: condition multiplication precedes clamp;
    /// critical/traits/spread/penalty remain rational until the caller's integer cast.
    #[allow(clippy::too_many_lines)] // Shared legacy/V6 draw order: hit, shield, critical.
    pub(super) fn auto_damage_v5(
        &mut self,
        source: TargetBinding,
        target: TargetBinding,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(AttackOutcome, u32), TickError> {
        use crate::domain::zone::scaled::round_div;
        use crate::domain::zone::skill_npc::{HitPosition, NpcPenaltyPath};
        let a = self.effective(source.entity)?;
        let b = self.effective(target.entity)?;
        let round = |v: i64| round_div(i128::from(v), i128::from(Q)).map_err(invariant);
        let accuracy = round(a.physical.accuracy().raw())?;
        let evasion = round(b.physical.evasion().raw())?;
        let base = 800_i128
            .checked_add(
                20_i128
                    .checked_mul(accuracy.checked_sub(evasion).ok_or_else(missing)?)
                    .ok_or_else(missing)?,
            )
            .ok_or_else(missing)?;
        // V6: actual victim-relative position (design §5.3); legacy keeps the front bonus.
        let v6 = self.phase4.is_some();
        let proximity = if v6 {
            self.proximity_v6(source.entity, target.entity)?
        } else {
            crate::domain::zone::item_combat::Proximity::Front
        };
        let position = match proximity {
            crate::domain::zone::item_combat::Proximity::Front => HitPosition::Front,
            crate::domain::zone::item_combat::Proximity::Side => HitPosition::Side,
            crate::domain::zone::item_combat::Proximity::Back => HitPosition::Back,
        };
        let condition = self
            .phase3
            .as_ref()
            .ok_or_else(missing)?
            .canonical
            .rules
            .npc()
            .hit_condition
            .multiplier(0, false, position)
            .map_err(invariant)?;
        let chance = ExactRatio::new(base, 1)
            .and_then(|v| v.checked_mul(condition))
            .and_then(ExactRatio::trunc)
            .map_err(invariant)?
            .clamp(200, 980);
        if i128::from(draw(self, DamageRollSite::AutoHit, target, 1000).map_err(invariant)?)
            > chance
        {
            return Ok((AttackOutcome::Miss, 0));
        }
        // L2Character.java:1324-1335: hit, then shield, then critical, then damage.
        let shield = if v6 {
            self.shield_v6(source, target, draw)?
        } else {
            crate::domain::zone::phase4::ShieldOutcome::None
        };
        let critical = draw(self, DamageRollSite::AutoCritical, target, 1000).map_err(invariant)?
            < a.physical.crit_permille();
        let outcome = if critical {
            AttackOutcome::Crit
        } else {
            AttackOutcome::Hit
        };
        if v6 {
            let damage =
                self.auto_damage_v6(source, target, &a, &b, proximity, shield, critical, draw)?;
            let damage = if self.bow_attacker_v6(source.entity) {
                self.bow_distance_v6(source.entity, target.entity, damage)?
            } else {
                damage
            };
            return Ok((outcome, damage));
        }
        let mut damage = q(a.physical.p_atk().raw())
            .map_err(invariant)?
            .checked_mul(ExactRatio::whole(76))
            .and_then(|v| v.checked_div(q(b.physical.p_def().raw())?))
            .map_err(invariant)?;
        if critical {
            let race_q = if self.entities.get(&source.entity).and_then(|e| e.combat.as_ref()).is_some_and(|c| matches!(&c.role, CombatRole::Player { progression: Some(p), .. } if p.identity.race == crate::domain::Race::DarkElf)) { 1_050_000 } else { Q };
            damage = damage
                .checked_mul(ExactRatio::whole(2))
                .and_then(|v| {
                    v.checked_mul(
                        self.damage_stat_v5(source.entity, &a, "critDmg", race_q)
                            .map_err(|_| StatError::Overflow)?,
                    )
                })
                .map_err(invariant)?;
            let extra = self
                .damage_stat_v5(source.entity, &a, "critDmgAdd", 0)?
                .checked_mul(ExactRatio::whole(77))
                .and_then(|v| v.checked_div(q(b.physical.p_def().raw())?))
                .map_err(invariant)?;
            damage = damage.checked_add(extra).map_err(invariant)?;
        }
        damage = damage
            .checked_mul(Self::attack_traits_v5(
                &a,
                &b,
                self.item_weapon_kind(
                    source.entity,
                    if self.sidecar(source.entity)?.player.is_some() {
                        "SWORD"
                    } else {
                        "NONE"
                    },
                )?,
            )?)
            .map_err(invariant)?;
        damage = damage
            .checked_mul(self.spread_v5(a.physical.random_damage(), target, draw)?)
            .map_err(invariant)?;
        damage = damage
            .checked_mul(self.item_pvp_damage_bonus(
                source.entity,
                target.entity,
                &a,
                "pvpPhysDmg",
            )?)
            .map_err(invariant)?;
        damage = damage
            .checked_mul(self.npc_penalty_v5(
                source.entity,
                target.entity,
                NpcPenaltyPath::Physical { critical },
            )?)
            .map_err(invariant)?;
        Ok((
            if critical {
                AttackOutcome::Crit
            } else {
                AttackOutcome::Hit
            },
            u32::try_from(damage.trunc().map_err(invariant)?.max(1)).map_err(invariant)?,
        ))
    }

    /// Formulas.java:667-733 ordinary auto (design §5.2 "Ordinary auto"): shield defence and
    /// `PvP` defence on `D'`, `P'=P×(charged?2:1)`, position `B`, H5 critical terms, then
    /// trait→spread→PvP damage→attribute→NPC penalty and max(1). A perfect block returns 1
    /// before the formula's spread draw.
    #[allow(clippy::too_many_arguments)] // Frozen caller inputs drawn in source order.
    fn auto_damage_v6(
        &mut self,
        source: TargetBinding,
        target: TargetBinding,
        a: &EffectiveStats,
        b: &EffectiveStats,
        proximity: crate::domain::zone::item_combat::Proximity,
        shield: crate::domain::zone::phase4::ShieldOutcome,
        critical: bool,
        draw: &mut DamageRoll<'_>,
    ) -> Result<u32, TickError> {
        use crate::domain::zone::phase4::ShieldOutcome;
        use crate::domain::zone::skill_npc::NpcPenaltyPath;
        if shield == ShieldOutcome::Perfect {
            return Ok(1);
        }
        let shield_defence = if shield == ShieldOutcome::Normal {
            self.damage_stat_v5(target.entity, b, "sDef", 0)?
                .trunc()
                .map_err(invariant)?
        } else {
            0
        };
        let defence = q(b.physical.p_def().raw())
            .map_err(invariant)?
            .checked_add(ExactRatio::new(shield_defence, 1).map_err(invariant)?)
            .and_then(|d| {
                d.checked_mul(
                    self.item_pvp_damage_bonus(target.entity, source.entity, b, "pvpPhysDef")
                        .map_err(|_| StatError::Overflow)?,
                )
            })
            .map_err(invariant)?;
        let attack = q(a.physical.p_atk().raw())
            .map_err(invariant)?
            .checked_mul(ExactRatio::whole(if self.attack_soulshot_v6(source.entity) {
                2
            } else {
                1
            }))
            .map_err(invariant)?;
        let base = attack
            .checked_mul(ExactRatio::whole(76))
            .and_then(|v| v.checked_mul(proximity.blow_damage()?))
            .and_then(|v| v.checked_div(defence))
            .map_err(invariant)?;
        let mut damage = if critical {
            let race_q = if self.entities.get(&source.entity).and_then(|e| e.combat.as_ref()).is_some_and(|c| matches!(&c.role, CombatRole::Player { progression: Some(p), .. } if p.identity.race == crate::domain::Race::DarkElf)) { 1_050_000 } else { Q };
            ExactRatio::whole(2)
                .checked_mul(self.damage_stat_v5(source.entity, a, "critDmg", race_q)?)
                .and_then(|v| {
                    v.checked_mul(
                        self.damage_stat_v5(source.entity, a, "critDmgPos", Q)
                            .map_err(|_| StatError::Overflow)?,
                    )
                })
                .and_then(|v| {
                    v.checked_mul(
                        self.damage_stat_v5(target.entity, b, "defCritDamage", Q)
                            .map_err(|_| StatError::Overflow)?,
                    )
                })
                .and_then(|v| v.checked_mul(base))
                .and_then(|v| {
                    v.checked_add(
                        self.damage_stat_v5(source.entity, a, "critDmgAdd", 0)
                            .map_err(|_| StatError::Overflow)?
                            .checked_mul(ExactRatio::whole(77))?
                            .checked_div(defence)?,
                    )
                })
                .and_then(|v| {
                    v.checked_add(
                        self.damage_stat_v5(target.entity, b, "defCritDamageAdd", 0)
                            .map_err(|_| StatError::Overflow)?,
                    )
                })
                .map_err(invariant)?
        } else {
            base
        };
        damage = damage
            .checked_mul(Self::attack_traits_v5(
                a,
                b,
                self.item_weapon_kind(
                    source.entity,
                    if self.sidecar(source.entity)?.player.is_some() {
                        "SWORD"
                    } else {
                        "NONE"
                    },
                )?,
            )?)
            .map_err(invariant)?;
        damage = damage
            .checked_mul(self.spread_v5(a.physical.random_damage(), target, draw)?)
            .and_then(|v| {
                v.checked_mul(
                    self.item_pvp_damage_bonus(source.entity, target.entity, a, "pvpPhysDmg")
                        .map_err(|_| StatError::Overflow)?,
                )
            })
            .and_then(|v| v.checked_mul(skill_math::attribute_bonus(0, 0, false, false)?))
            .map_err(invariant)?;
        damage = damage
            .checked_mul(self.npc_penalty_v5(
                source.entity,
                target.entity,
                NpcPenaltyPath::Physical { critical },
            )?)
            .map_err(invariant)?;
        u32::try_from(damage.trunc().map_err(invariant)?.max(1)).map_err(invariant)
    }
}
