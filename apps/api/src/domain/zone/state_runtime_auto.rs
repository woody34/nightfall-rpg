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
        let mut due: Vec<_> = self
            .entities
            .values()
            .filter_map(|e| {
                let swing = e.combat.as_ref()?.swing?;
                (swing.impact <= tick).then_some((e.id, Some(swing)))
            })
            .collect();
        // I3: a stored second dual subhit belongs to an earlier attack than any new swing of
        // the same attacker, so it lands first (attacker order, then subhit order).
        if let Some(items) = self.phase4.as_ref() {
            for (attacker, plan) in &items.canonical.attack_plans {
                let pending = usize::try_from(plan.next_hit)
                    .ok()
                    .filter(|index| *index > 0)
                    .and_then(|index| plan.hits.get(index));
                if pending.is_some_and(|hit| hit.at <= tick && hit.prepared.is_some()) {
                    due.push((*attacker, None));
                }
            }
        }
        due.sort_by_key(|(id, swing)| (*id, swing.is_some()));
        // BTreeMap traversal retains the legacy attacker EntityId order (design phase6).
        for (id, swing) in due {
            let Some(swing) = swing else {
                self.land_dual_second_v6(tick, id, events, &mut draw_damage_v5)?;
                continue;
            };
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
                // Source discharges at launch when hit1 OR hit2 lands (L2Character.java:951-958);
                // a dual's second outcome is already resolved in its plan.
                let pending_landed = self
                    .phase4
                    .as_ref()
                    .and_then(|items| items.canonical.attack_plans.get(&id))
                    .filter(|plan| plan.start == swing.start)
                    .and_then(|plan| plan.hits.get(usize::try_from(plan.next_hit).ok()?))
                    .and_then(|hit| hit.prepared.as_ref())
                    .is_some_and(|hit| hit.outcome != AttackOutcome::Miss);
                let landed = pending_landed
                    || events
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
        let style = self.attack_style_v6(id);
        if style == AttackStyle::Dual {
            return self.land_dual_first_v6(tick, id, swing, source, target, events, draw);
        }
        let (outcome, rolled) = if style == AttackStyle::Pole {
            // doAttackHitByPole primary: doAttackHitSimple at 100%, D25 base = trunc(formula).
            if self.auto_hit_v5(source, target, draw)? {
                let word = self.rng.get_word_pos();
                let (outcome, damage, _, exact) = self.auto_resolve_v5(source, target, draw)?;
                if let Some(exact) = exact {
                    use crate::domain::zone::state::item_evidence_hooks::ratio_json as r;
                    let percent = ExactRatio::whole(100);
                    self.item_formula_v6(
                        id,
                        "pole_damage",
                        serde_json::json!({"formula": r(exact), "percent": r(percent), "primary": true}),
                        serde_json::json!({"base": damage, "percent": r(percent), "damage": damage}),
                        &format!("pole:{id}:{word}"),
                        Some(self.attack_context_v6(id, target.entity)),
                    );
                }
                (outcome, damage)
            } else {
                (AttackOutcome::Miss, 0)
            }
        } else {
            self.auto_damage_v5(source, target, draw)?
        };
        self.apply_auto_hit_v5(tick, id, source, target, outcome, rolled, events, draw)?;
        if style == AttackStyle::Pole {
            self.land_pole_secondaries_v6(tick, id, source, target, events, draw)?;
        }
        Ok(())
    }

    /// Recipient application shared by every auto subhit (single, dual halves, pole
    /// recipients): status/death hooks, hate, attack break and the public result.
    #[allow(clippy::too_many_arguments)] // Frozen subhit identity and its resolved outcome.
    pub(super) fn apply_auto_hit_v5(
        &mut self,
        tick: Tick,
        id: EntityId,
        source: TargetBinding,
        target: TargetBinding,
        outcome: AttackOutcome,
        rolled: u32,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
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
        if !self.auto_hit_v5(source, target, draw)? {
            return Ok((AttackOutcome::Miss, 0));
        }
        self.auto_resolve_v5(source, target, draw)
            .map(|(outcome, damage, _, _)| (outcome, damage))
    }

    /// Formulas.calcHitMiss: one hit draw; `true` when the hit lands.
    pub(super) fn auto_hit_v5(
        &mut self,
        source: TargetBinding,
        target: TargetBinding,
        draw: &mut DamageRoll<'_>,
    ) -> Result<bool, TickError> {
        use crate::domain::zone::scaled::round_div;
        use crate::domain::zone::skill_npc::HitPosition;
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
        Ok(
            i128::from(draw(self, DamageRollSite::AutoHit, target, 1000).map_err(invariant)?)
                <= chance,
        )
    }

    /// Landed subhit: shield, critical, then the caller's damage (L2Character.java:1324-1335).
    #[allow(clippy::too_many_lines)] // Shared legacy/V6 order: shield, critical, damage.
    pub(super) fn auto_resolve_v5(
        &mut self,
        source: TargetBinding,
        target: TargetBinding,
        draw: &mut DamageRoll<'_>,
    ) -> Result<AutoResolution, TickError> {
        use crate::domain::zone::skill_npc::NpcPenaltyPath;
        let a = self.effective(source.entity)?;
        let b = self.effective(target.entity)?;
        let v6 = self.phase4.is_some();
        let proximity = if v6 {
            self.proximity_v6(source.entity, target.entity)?
        } else {
            crate::domain::zone::item_combat::Proximity::Front
        };
        if v6 && self.items_evidence_enabled() && self.phase4_player_v6(source.entity) {
            self.position_evidence_v6(source.entity, target.entity, proximity)?;
        }
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
            let (damage, exact) =
                self.auto_damage_v6(source, target, &a, &b, proximity, shield, critical, draw)?;
            let damage = if self.bow_attacker_v6(source.entity) {
                let scaled = self.bow_distance_v6(source.entity, target.entity, damage)?;
                self.bow_damage_evidence_v6(source.entity, target.entity, exact, damage, scaled)?;
                scaled
            } else {
                damage
            };
            return Ok((outcome, damage, shield, Some(exact)));
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
            shield,
            None,
        ))
    }

    /// Formulas.java:667-733 ordinary auto (design §5.2 "Ordinary auto"): shield defence and
    /// `PvP` defence on `D'`, `P'=P×(charged?2:1)`, position `B`, H5 critical terms, then
    /// trait→spread→PvP damage→attribute→NPC penalty and max(1). A perfect block returns 1
    /// before the formula's spread draw. Returns the caller's integer and the exact formula
    /// value (after its max(1)) for the dual/pole/bow caller truncations.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Frozen caller inputs drawn in source order.
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
    ) -> Result<(u32, ExactRatio), TickError> {
        use crate::domain::zone::item_combat::Proximity;
        use crate::domain::zone::phase4::ShieldOutcome;
        use crate::domain::zone::skill_npc::NpcPenaltyPath;
        let word = self.rng.get_word_pos();
        let one = ExactRatio::whole(1);
        let shot = self.attack_soulshot_v6(source.entity);
        let pvp_defence =
            self.item_pvp_damage_bonus(target.entity, source.entity, b, "pvpPhysDef")?;
        let p_def = q(b.physical.p_def().raw()).map_err(invariant)?;
        let p_atk = q(a.physical.p_atk().raw()).map_err(invariant)?;
        let perfect = shield == ShieldOutcome::Perfect;
        let shield_defence = if shield == ShieldOutcome::Normal {
            self.damage_stat_v5(target.entity, b, "sDef", 0)?
                .trunc()
                .map_err(invariant)?
        } else {
            0
        };
        let shield_defence = ExactRatio::new(shield_defence, 1).map_err(invariant)?;
        let race_q = if self.entities.get(&source.entity).and_then(|e| e.combat.as_ref()).is_some_and(|c| matches!(&c.role, CombatRole::Player { progression: Some(p), .. } if p.identity.race == crate::domain::Race::DarkElf)) { 1_050_000 } else { Q };
        let crit_dmg = self.damage_stat_v5(source.entity, a, "critDmg", race_q)?;
        let crit_pos = self.damage_stat_v5(source.entity, a, "critDmgPos", Q)?;
        let def_crit_dmg = self.damage_stat_v5(target.entity, b, "defCritDamage", Q)?;
        let crit_add = self.damage_stat_v5(source.entity, a, "critDmgAdd", 0)?;
        let def_crit_add = self.damage_stat_v5(target.entity, b, "defCritDamageAdd", 0)?;
        let traits = Self::attack_traits_v5(
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
        )?;
        let pvp = self.item_pvp_damage_bonus(source.entity, target.entity, a, "pvpPhysDmg")?;
        let attribute = skill_math::attribute_bonus(0, 0, false, false).map_err(invariant)?;
        let npc = self.npc_penalty_v5(
            source.entity,
            target.entity,
            NpcPenaltyPath::Physical { critical },
        )?;
        let radius = a.physical.random_damage();
        let (exact, spread) = if perfect {
            (one, None)
        } else {
            let defence = p_def
                .checked_add(shield_defence)
                .and_then(|d| d.checked_mul(pvp_defence))
                .map_err(invariant)?;
            let attack = p_atk
                .checked_mul(ExactRatio::whole(if shot { 2 } else { 1 }))
                .map_err(invariant)?;
            let base = attack
                .checked_mul(ExactRatio::whole(76))
                .and_then(|v| v.checked_mul(proximity.blow_damage()?))
                .and_then(|v| v.checked_div(defence))
                .map_err(invariant)?;
            let mut damage = if critical {
                ExactRatio::whole(2)
                    .checked_mul(crit_dmg)
                    .and_then(|v| v.checked_mul(crit_pos))
                    .and_then(|v| v.checked_mul(def_crit_dmg))
                    .and_then(|v| v.checked_mul(base))
                    .and_then(|v| {
                        v.checked_add(
                            crit_add
                                .checked_mul(ExactRatio::whole(77))?
                                .checked_div(defence)?,
                        )
                    })
                    .and_then(|v| v.checked_add(def_crit_add))
                    .map_err(invariant)?
            } else {
                base
            };
            damage = damage.checked_mul(traits).map_err(invariant)?;
            let spread = self.spread_v5(radius, target, draw)?;
            damage = damage
                .checked_mul(spread)
                .and_then(|v| v.checked_mul(pvp))
                .and_then(|v| v.checked_mul(attribute))
                .and_then(|v| v.checked_mul(npc))
                .map_err(invariant)?;
            let exact = if damage.checked_cmp(one).map_err(invariant)?.is_lt() {
                one
            } else {
                damage
            };
            (exact, Some(spread))
        };
        if self.items_evidence_enabled() && self.phase4_player_v6(source.entity) {
            use crate::domain::zone::state::item_evidence_hooks::ratio_json as r;
            let resolution = format!("auto:{}:{word}", source.entity);
            let proximity = match proximity {
                Proximity::Front => "front",
                Proximity::Side => "side",
                Proximity::Back => "back",
            };
            // PvP defence multiplies both defence terms: (P.Def + sDef)·k = P.Def·k + sDef·k.
            let scaled = |v: ExactRatio| v.checked_mul(pvp_defence).map_err(invariant);
            let inputs = serde_json::json!({
                "p_atk": r(p_atk), "defence": r(scaled(p_def)?), "power": 0, "caller": "auto",
                "shot": shot, "proximity": proximity, "shield_def": r(scaled(shield_defence)?),
                "perfect": perfect, "critical": critical, "crit_dmg": r(crit_dmg),
                "crit_pos": r(crit_pos), "def_crit_dmg": r(def_crit_dmg), "crit_add": r(crit_add),
                "def_crit_add": r(def_crit_add), "trait": r(traits),
                "spread": r(spread.unwrap_or(one)), "pvp": r(pvp), "attribute": r(attribute),
                "npc": r(npc)});
            let context = self.attack_context_v6(source.entity, target.entity);
            self.item_formula_v6(
                source.entity,
                "physical_damage",
                inputs,
                r(exact),
                &resolution,
                Some(context),
            );
            if let Some(spread) = spread {
                let level = a.physical.level();
                let residue = if radius == 0 {
                    None
                } else {
                    let percent = spread
                        .checked_mul(ExactRatio::whole(100))
                        .and_then(ExactRatio::trunc)
                        .map_err(invariant)?;
                    Some(
                        i64::try_from(percent)
                            .ok()
                            .and_then(|p| p.checked_sub(100))
                            .and_then(|p| p.checked_add(i64::from(radius)))
                            .ok_or_else(missing)?,
                    )
                };
                // D24: an unarmed player's radius is recomputed by the oracle from its level.
                // The radius comes from a template (equipped or D84 fists weapon); only a
                // fistless class leaves the oracle to recompute 5+isqrt(level).
                let template = (self.held_weapon_v6(source.entity).is_some()
                    || self.unarmed_fists_v6(source.entity)?.is_some())
                .then_some(radius);
                let offset = residue
                    .map(|value| value.checked_sub(i64::from(radius)).ok_or_else(missing))
                    .transpose()?;
                let bound = u64::from(radius)
                    .checked_mul(2)
                    .and_then(|v| v.checked_add(1))
                    .ok_or_else(missing)?;
                let actual = serde_json::json!({"radius": radius, "bound": bound,
                    "offset": offset, "multiplier": residue.map(|_| r(spread))});
                self.item_formula_v6(
                    source.entity,
                    "fist_spread",
                    serde_json::json!({"level": level, "residue": residue, "template_radius": template}),
                    actual,
                    &resolution,
                    Some(self.attack_context_v6(source.entity, target.entity)),
                );
            }
        }
        Ok((u32::try_from(exact.trunc().map_err(invariant)?).map_err(invariant)?, exact))
    }
}

/// L2Character.java:897-934 attack routing for a V6 player (NPCs and legacy stay simple).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::domain::zone) enum AttackStyle {
    /// `doAttackHitSimple` (and the bow/crossbow callers).
    Simple,
    /// `doAttackHitByDual`: DUAL/DUALFIST/DUALDAGGER and the player FIST fall-through.
    Dual,
    /// `doAttackHitByPole`: primary plus percentage-scaled secondaries.
    Pole,
}

impl ZoneState {
    /// Player weapon routing; an unarmed player attacks with the template FIST type, which
    /// falls through to the dual handler (L2Character.java:923-935).
    pub(in crate::domain::zone) fn attack_style_v6(&self, id: EntityId) -> AttackStyle {
        if self
            .phase4
            .as_ref()
            .is_none_or(|items| !items.canonical.players.contains_key(&id))
        {
            return AttackStyle::Simple;
        }
        match self
            .held_weapon_v6(id)
            .map(|(_, definition)| definition.weapon_type.as_str())
        {
            None | Some("FIST" | "DUAL" | "DUALFIST" | "DUALDAGGER") => AttackStyle::Dual,
            Some("POLE") => AttackStyle::Pole,
            Some(_) => AttackStyle::Simple,
        }
    }

    /// I3 (design §5.4): when the first subhit is due, draw hit1 and hit2, then shield/
    /// critical/spread for each landed hit in order (L2Character.java:1137-1179); each half
    /// is `trunc(formula) / 2`. The first applies now; the second waits in the plan.
    #[allow(clippy::too_many_arguments)] // Frozen swing identity and the shared draw source.
    fn land_dual_first_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        swing: Swing,
        source: TargetBinding,
        target: TargetBinding,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
        let hit1 = self.auto_hit_v5(source, target, draw)?;
        let hit2 = self.auto_hit_v5(source, target, draw)?;
        let mut halves = Vec::with_capacity(2);
        for (index, hit) in [hit1, hit2].into_iter().enumerate() {
            halves.push(if hit {
                let word = self.rng.get_word_pos();
                let (outcome, damage, shield, exact) =
                    self.auto_resolve_v5(source, target, draw)?;
                // L2Character.java:1155-1171: (int)calcPhysDam then integer /2.
                self.subhit_evidence_v6(
                    id,
                    target.entity,
                    "dual_damage",
                    exact,
                    false,
                    &format!("dual:{id}:{word}:{index}"),
                )?;
                (outcome, damage / 2, shield)
            } else {
                self.subhit_evidence_v6(
                    id,
                    target.entity,
                    "dual_damage",
                    None,
                    true,
                    &format!("dual:{id}:miss:{}:{index}", swing.start.0),
                )?;
                (AttackOutcome::Miss, 0, crate::domain::zone::phase4::ShieldOutcome::None)
            });
        }
        let [(first, first_damage, _), (second, second_damage, second_shield)] = halves.as_slice()
        else {
            return Err(missing());
        };
        let plan = self
            .phase4
            .as_mut()
            .ok_or_else(missing)?
            .canonical
            .attack_plans
            .get_mut(&id)
            .filter(|plan| plan.start == swing.start && plan.hits.len() == 2 && plan.next_hit == 0)
            .ok_or_else(missing)?;
        let pending = plan.hits.get_mut(1).ok_or_else(missing)?;
        pending.prepared = Some(crate::domain::zone::phase4::PreparedItemHit {
            outcome: *second,
            shield: *second_shield,
            damage: *second_damage,
        });
        plan.next_hit = 1;
        self.apply_auto_hit_v5(tick, id, source, target, *first, *first_damage, events, draw)
    }

    /// Second dual subhit at its own deadline: the stored result applies to the planned
    /// life only while attacker and recipient are alive (`HitTask`; no new damage draws).
    pub(in crate::domain::zone) fn land_dual_second_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
        let Some(plan) = self
            .phase4
            .as_mut()
            .and_then(|items| items.canonical.attack_plans.remove(&id))
        else {
            return Ok(());
        };
        let hit = usize::try_from(plan.next_hit)
            .ok()
            .and_then(|index| plan.hits.get(index))
            .ok_or_else(missing)?;
        let prepared = hit.prepared.as_ref().ok_or_else(missing)?;
        let source = self.binding(id).ok_or_else(missing)?;
        let attacker_alive = self
            .entities
            .get(&id)
            .is_some_and(|e| !e.targeting.dead && e.combat.is_some());
        let target_alive = self.binding(hit.target.entity) == Some(hit.target)
            && self
                .entities
                .get(&hit.target.entity)
                .is_some_and(|e| !e.targeting.dead);
        if source != plan.attacker || !attacker_alive || !target_alive {
            return Ok(());
        }
        self.apply_auto_hit_v5(
            tick,
            id,
            source,
            hit.target,
            prepared.outcome,
            prepared.damage,
            events,
            draw,
        )
    }

    /// `doAttackHitByPole` secondaries (D25, design §5.4): candidates in entity-ordinal then
    /// `EntityId` order, inside `range + collision/2` and the weapon attack cone around the
    /// primary-target heading, alive and auto-attackable; each attempted candidate (hit or
    /// miss) advances 85% by ÷23/20 and the scan breaks once the count exceeds atkCountMax.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // Source scan, eligibility and per-recipient caller.
    fn land_pole_secondaries_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        source: TargetBinding,
        primary: TargetBinding,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
        let a = self.effective(id)?;
        let maximum = self
            .damage_stat_v5(id, &a, "atkCountMax", Q)?
            .trunc()
            .map_err(invariant)?;
        let (_, definition) = self.held_weapon_v6(id).ok_or_else(missing)?;
        // CharStat.java:438-447: the weapon base angle, without stat modifiers.
        let angle = definition.attack_angle;
        let attacker = self.entities.get(&id).ok_or_else(missing)?;
        let combat = attacker.combat.as_ref().ok_or_else(missing)?;
        let (origin, range, radius) = (attacker.pos, combat.attack_range, combat.collision_radius);
        let primary_pos = self.entities.get(&primary.entity).ok_or_else(missing)?.pos;
        // Source sets the heading toward the primary target at launch (doAttack:888).
        let heading =
            crate::domain::zone::item_angles::FacingVector::EAST.moved(origin, primary_pos);
        let items = self.phase4.as_ref().ok_or_else(missing)?;
        let certificate = items
            .rules
            .parts()
            .angles
            .get(usize::try_from(angle).map_err(invariant)?)
            .ok_or_else(missing)?;
        let bound = items.rules.parts().coordinate_bound_millitiles;
        let mut candidates = Vec::new();
        for other in self.aoi.in_aoi(origin) {
            if other == id || other == primary.entity {
                continue;
            }
            let Some(entity) = self.entities.get(&other) else {
                continue;
            };
            let Some(c) = entity.combat.as_ref() else {
                continue;
            };
            if entity.targeting.dead {
                continue;
            }
            // Util.checkIfInRange: d − (rA + rB)/2 ≤ range, compared exactly as
            // 4d² ≤ (2·range + rA + rB)².
            let reach = i128::from(range.raw())
                .checked_mul(2)
                .and_then(|v| v.checked_add(i128::from(radius.raw())))
                .and_then(|v| v.checked_add(i128::from(c.collision_radius.raw())))
                .ok_or_else(missing)?;
            let distance = i128::try_from(origin.distance_sq(entity.pos)).map_err(invariant)?;
            if distance.checked_mul(4).ok_or_else(missing)?
                > reach.checked_mul(reach).ok_or_else(missing)?
            {
                continue;
            }
            let dx = i64::from(entity.pos.x.raw())
                .checked_sub(i64::from(origin.x.raw()))
                .ok_or_else(missing)?;
            let dy = i64::from(entity.pos.y.raw())
                .checked_sub(i64::from(origin.y.raw()))
                .ok_or_else(missing)?;
            if !heading
                .within(dx, dy, certificate, bound)
                .map_err(invariant)?
            {
                continue;
            }
            let binding = TargetBinding {
                entity: other,
                incarnation: c.incarnation,
            };
            if self.hostility_v5(tick, source, binding).is_err() {
                continue;
            }
            // L2PcInstance.isAutoAttackable: another player only while flagged or chaotic.
            if entity.kind == EntityKind::Player
                && !self.sidecar(other)?.player.as_ref().is_some_and(|p| {
                    p.pvp.karma > 0 || p.pvp.flag_until.is_some_and(|until| until > tick)
                })
            {
                continue;
            }
            let ordinal = self.sidecar(other)?.ordinal;
            candidates.push((ordinal, other, binding));
        }
        candidates.sort_by_key(|(ordinal, entity, _)| (*ordinal, *entity));
        let mut percent = ExactRatio::whole(85);
        let mut count = 0_i128;
        for (_, _, target) in candidates {
            // A previous recipient's death hook may have removed this candidate.
            if self.binding(target.entity) != Some(target)
                || self
                    .entities
                    .get(&target.entity)
                    .is_none_or(|e| e.targeting.dead)
            {
                continue;
            }
            let (outcome, damage) = if self.auto_hit_v5(source, target, draw)? {
                let word = self.rng.get_word_pos();
                let (outcome, base, _, exact) = self.auto_resolve_v5(source, target, draw)?;
                let scaled = ExactRatio::whole(i64::from(base))
                    .checked_mul(percent)
                    .and_then(|v| v.checked_div(ExactRatio::whole(100)))
                    .and_then(ExactRatio::trunc)
                    .map_err(invariant)?;
                let scaled = u32::try_from(scaled).map_err(invariant)?;
                if let Some(exact) = exact {
                    use crate::domain::zone::state::item_evidence_hooks::ratio_json as r;
                    self.item_formula_v6(
                        id,
                        "pole_damage",
                        serde_json::json!({"formula": r(exact), "percent": r(percent), "primary": false}),
                        serde_json::json!({"base": base, "percent": r(percent), "damage": scaled}),
                        &format!("pole:{id}:{word}"),
                        Some(self.attack_context_v6(id, target.entity)),
                    );
                }
                (outcome, scaled)
            } else {
                (AttackOutcome::Miss, 0)
            };
            self.apply_auto_hit_v5(tick, id, source, target, outcome, damage, events, draw)?;
            percent = percent
                .checked_mul(ExactRatio::new(20, 23).map_err(invariant)?)
                .map_err(invariant)?;
            count = count.checked_add(1).ok_or_else(missing)?;
            if count > maximum {
                break;
            }
        }
        Ok(())
    }
}

/// Landed V6 subhit: outcome, caller integer, shield outcome and the exact formula value.
pub(super) type AutoResolution = (
    AttackOutcome,
    u32,
    crate::domain::zone::phase4::ShieldOutcome,
    Option<ExactRatio>,
);

impl ZoneState {
    /// D46 `position`: victim facing and victim→attacker vector (design §5.3).
    fn position_evidence_v6(
        &mut self,
        attacker: EntityId,
        target: EntityId,
        proximity: crate::domain::zone::item_combat::Proximity,
    ) -> Result<(), TickError> {
        use crate::domain::zone::item_combat::Proximity;
        let facing = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.facing.get(&target))
            .ok_or_else(missing)?
            .components();
        let a = self.entities.get(&attacker).ok_or_else(missing)?.pos;
        let b = self.entities.get(&target).ok_or_else(missing)?.pos;
        let relative = (
            i64::from(a.x.raw())
                .checked_sub(i64::from(b.x.raw()))
                .ok_or_else(missing)?,
            i64::from(a.y.raw())
                .checked_sub(i64::from(b.y.raw()))
                .ok_or_else(missing)?,
        );
        let actual = match proximity {
            Proximity::Front => "front",
            Proximity::Side => "side",
            Proximity::Back => "back",
        };
        let word = self.rng.get_word_pos();
        self.item_formula_v6(
            attacker,
            "position",
            serde_json::json!({"facing": [facing.0, facing.1], "relative": [relative.0, relative.1]}),
            serde_json::json!(actual),
            &format!("position:{attacker}:{target}:{word}"),
            Some(self.attack_context_v6(attacker, target)),
        );
        Ok(())
    }

    /// D46 `dual_damage` per subhit (L2Character.java:1137-1179).
    #[allow(clippy::too_many_arguments)] // Subhit identity and its formula value.
    fn subhit_evidence_v6(
        &mut self,
        attacker: EntityId,
        target: EntityId,
        formula: &str,
        exact: Option<ExactRatio>,
        miss: bool,
        resolution: &str,
    ) -> Result<(), TickError> {
        use crate::domain::zone::state::item_evidence_hooks::ratio_json as r;
        if !self.items_evidence_enabled() {
            return Ok(());
        }
        let value = exact.unwrap_or(ExactRatio::whole(0));
        let base = if miss {
            0
        } else {
            u32::try_from(value.trunc().map_err(invariant)?).map_err(invariant)?
        };
        let actual = serde_json::json!({"base": base, "damage": base / 2,
            "draws": if miss { serde_json::json!([]) } else { serde_json::json!(["shield", "critical", "spread"]) }});
        self.item_formula_v6(
            attacker,
            formula,
            serde_json::json!({"formula": r(value), "miss": miss}),
            actual,
            resolution,
            Some(self.attack_context_v6(attacker, target)),
        );
        Ok(())
    }

    /// D46/D80 `bow_damage` (D17): inputs are the exact caller formula and the milli-tile
    /// displacement; the actual reproduces the frozen root certificate exactly.
    fn bow_damage_evidence_v6(
        &mut self,
        attacker: EntityId,
        target: EntityId,
        exact: ExactRatio,
        base: u32,
        damage: u32,
    ) -> Result<(), TickError> {
        use crate::domain::zone::state::item_evidence_hooks::{bow_damage_actual, ratio_json as r};
        if !self.items_evidence_enabled() || !self.phase4_player_v6(attacker) {
            return Ok(());
        }
        let a = self.entities.get(&attacker).ok_or_else(missing)?.pos;
        let b = self.entities.get(&target).ok_or_else(missing)?.pos;
        let dx = i64::from(a.x.raw())
            .checked_sub(i64::from(b.x.raw()))
            .ok_or_else(missing)?;
        let dy = i64::from(a.y.raw())
            .checked_sub(i64::from(b.y.raw()))
            .ok_or_else(missing)?;
        let actual = bow_damage_actual(u64::from(base), dx, dy).ok_or_else(missing)?;
        // The engine's isqrt formula and the certificate must agree on the caller integer.
        if actual.get("damage") != Some(&serde_json::json!(damage)) {
            return Err(missing());
        }
        let word = self.rng.get_word_pos();
        self.item_formula_v6(
            attacker,
            "bow_damage",
            serde_json::json!({"formula": r(exact), "dx_milli": dx, "dy_milli": dy}),
            actual,
            &format!("bow:{attacker}:{word}"),
            Some(self.attack_context_v6(attacker, target)),
        );
        Ok(())
    }
}
