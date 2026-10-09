//! The combat phases of [`ZoneState::run_tick`] and the combat commands (plan §3.2, Stories
//! E2.3 and E2.5). Tick order: commands → chase → movement → due impacts by attacker id
//! (with their immediate damage and death consequences) → start eligible swings → AOI output.
//!
//! RNG draws happen only at a valid impact, in this order: hit roll, then on a hit the crit
//! roll and (radius > 0) the damage spread. Cancelled or invalid swings draw nothing.

use super::super::combat::{CombatRole, HateEntry, Swing, SwingCancel};
use super::super::combat_math::{
    attack_timing, crit_lands, damage_hate, hit_chance_permille, hit_lands, physical_damage,
};
use super::super::command::{AttackOutcome, RejectReason, ZoneEvent};
use super::super::entity::{Entity, EntityId, EntityKind, Tick};
use super::super::fixed::Vec2Fixed;
use super::super::stat_rules::FormulaConstants;
use super::super::stat_sheet::StatSheet;
use super::{stats_changed, ZoneState};

/// What [`ZoneState::engagement`] found for an attacker's current target.
struct Engagement {
    target: EntityId,
    incarnation: u32,
    pos: Vec2Fixed,
    in_reach: bool,
}

impl ZoneState {
    /// `Attack`: validates the actor and its current target, then enables auto-attack.
    /// Enabling twice is a no-op, so a repeat never resets the cycle or adds a swing.
    pub(super) fn attack(
        &mut self,
        tick: Tick,
        entity: EntityId,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let actor = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let combat = actor.combat.as_ref().ok_or(RejectReason::NotPermitted)?;
        let target_id = actor.targeting.target.ok_or(RejectReason::UnknownEntity)?;
        let target = self
            .entities
            .get(&target_id)
            .ok_or(RejectReason::UnknownEntity)?;
        if !may_attack(actor, target) {
            return Err(RejectReason::NonAttackableTarget);
        }
        if !self.aoi.in_aoi(actor.pos).any(|id| id == target_id) {
            return Err(RejectReason::TargetNotInAoi);
        }
        if target.combat.as_ref().is_some_and(|c| c.protected_at(tick)) {
            return Err(RejectReason::Protected);
        }
        if combat.auto_attack {
            return Ok(Vec::new());
        }
        if let Some(c) = self
            .entities
            .get_mut(&entity)
            .and_then(|e| e.combat.as_mut())
        {
            c.auto_attack = true;
            // Attacking ends respawn protection (E2.4).
            c.protected_until = None;
        }
        Ok(Vec::new())
    }

    /// `StopAttack`: disables auto-attack and cancels the pending swing. Repeats are no-ops.
    pub(super) fn stop_attack(
        &mut self,
        tick: Tick,
        entity: EntityId,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let actor = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if actor.combat.is_none() {
            return Err(RejectReason::NotPermitted);
        }
        let mut events = Vec::new();
        self.disengage(tick, entity, SwingCancel::Stopped, &mut events);
        Ok(events)
    }

    /// `AddAggro`: an auto/social aggro entry worth 1 hate (plan §3.1), then target
    /// re-selection. For the NPC AI (E3.2).
    pub(super) fn add_aggro(
        &mut self,
        tick: Tick,
        npc: EntityId,
        target: EntityId,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let npc_e = self.entities.get(&npc).ok_or(RejectReason::UnknownEntity)?;
        if npc_e.kind != EntityKind::Npc || npc_e.combat.is_none() {
            return Err(RejectReason::NotPermitted);
        }
        if npc_e.targeting.dead {
            return Err(RejectReason::DeadActor);
        }
        if self.is_returning(npc) {
            return Err(RejectReason::NotPermitted);
        }
        let prey = self
            .entities
            .get(&target)
            .ok_or(RejectReason::UnknownEntity)?;
        if !may_attack(npc_e, prey) {
            return Err(RejectReason::NonAttackableTarget);
        }
        if prey.combat.as_ref().is_some_and(|c| c.protected_at(tick)) {
            return Err(RejectReason::Protected);
        }
        let mut events = Vec::new();
        self.add_hate(tick, npc, target, HateEntry { hate: 1, damage: 0 }, &mut events);
        Ok(events)
    }

    /// Turns auto-attack off, cancels a pending swing and stops a chase. The selection stays.
    pub(super) fn disengage(
        &mut self,
        tick: Tick,
        entity: EntityId,
        reason: SwingCancel,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(e) = self.entities.get_mut(&entity) else {
            return;
        };
        let Some(c) = e.combat.as_mut() else {
            return;
        };
        c.auto_attack = false;
        if let Some(swing) = c.swing.take() {
            events.push(ZoneEvent::AttackCancelled {
                tick,
                attacker: entity,
                target: swing.target,
                reason,
            });
        }
        if c.chasing {
            c.chasing = false;
            if e.dest.take().is_some() {
                events.push(ZoneEvent::EntityMove {
                    tick,
                    entity,
                    pos: e.pos,
                    dest: None,
                    speed: e.speed,
                });
            }
        }
    }

    /// `gone` died or left: it leaves every hate ledger, and everyone holding it as target
    /// loses the target and its swing; NPCs then re-select (plan E2.3, E2.5).
    pub(super) fn release_target(
        &mut self,
        tick: Tick,
        gone: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) {
        for (npc, ledger) in &mut self.hate {
            if ledger.forget(gone) {
                events.push(ZoneEvent::HateChanged {
                    tick,
                    npc: *npc,
                    target: gone,
                    hate: 0,
                    damage: 0,
                });
            }
        }
        self.hate.retain(|_, ledger| !ledger.is_empty());
        let holders: Vec<EntityId> = self
            .entities
            .values()
            .filter(|e| {
                e.targeting.target == Some(gone)
                    || e.combat
                        .as_ref()
                        .and_then(|c| c.swing)
                        .is_some_and(|s| s.target == gone)
            })
            .map(|e| e.id)
            .collect();
        for holder in holders {
            self.disengage(tick, holder, SwingCancel::TargetLost, events);
            let Some(e) = self.entities.get_mut(&holder) else {
                continue;
            };
            if e.targeting.target.take().is_some() {
                events.push(ZoneEvent::TargetChanged {
                    tick,
                    entity: holder,
                    target: None,
                });
            }
            if e.kind == EntityKind::Npc {
                self.reselect(tick, holder, events);
            }
        }
    }

    /// Adds hate (and damage) to `npc`'s ledger for `target`, then re-selects its target.
    pub(super) fn add_hate(
        &mut self,
        tick: Tick,
        npc: EntityId,
        target: EntityId,
        added: HateEntry,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(rules) = self.rules.clone() else {
            return;
        };
        let row = self.hate.entry(npc).or_default().add(
            rules.constants(),
            target,
            added.hate,
            added.damage,
        );
        events.push(ZoneEvent::HateChanged {
            tick,
            npc,
            target,
            hate: row.hate,
            damage: row.damage,
        });
        self.reselect(tick, npc, events);
    }

    /// Points `npc` at its most hated eligible entity (ties keep the current target, then the
    /// lowest id) and enables auto-attack; with nobody eligible it stands down.
    pub(super) fn reselect(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(me) = self.entities.get(&npc) else {
            return;
        };
        if me.targeting.dead || me.combat.is_none() {
            return;
        }
        let current = me.targeting.target;
        let chosen = self.hate.get(&npc).and_then(|ledger| {
            ledger.most_hated(current, |id| {
                self.entities.get(&id).is_some_and(|t| {
                    may_attack(me, t) && !t.combat.as_ref().is_some_and(|c| c.protected_at(tick))
                })
            })
        });
        if chosen != current {
            self.disengage(tick, npc, SwingCancel::TargetChanged, events);
            if let Some(e) = self.entities.get_mut(&npc) {
                e.targeting.target = chosen;
            }
            events.push(ZoneEvent::TargetChanged {
                tick,
                entity: npc,
                target: chosen,
            });
        }
        if let Some(c) = self.entities.get_mut(&npc).and_then(|e| e.combat.as_mut()) {
            c.auto_attack = chosen.is_some();
        }
        // E3.2 hook: intention follows the target (state_ai.rs).
        self.ai_target_changed(tick, npc, chosen, events);
    }

    /// The attacker's current target, if it may still be fought: alive, attackable by this
    /// attacker, unprotected and, for players, inside the attacker's AOI.
    fn engagement(&self, tick: Tick, attacker: &Entity) -> Option<Engagement> {
        let combat = attacker.combat.as_ref()?;
        let target_id = attacker.targeting.target?;
        let target = self.entities.get(&target_id)?;
        let tc = target.combat.as_ref()?;
        if !may_attack(attacker, target) || tc.protected_at(tick) {
            return None;
        }
        if attacker.kind == EntityKind::Player
            && !self.aoi.in_aoi(attacker.pos).any(|id| id == target_id)
        {
            return None;
        }
        let reach = combat.attack_range.saturating_add(tc.collision_radius);
        Some(Engagement {
            target: target_id,
            incarnation: tc.incarnation,
            pos: target.pos,
            in_reach: attacker.pos.within(target.pos, reach),
        })
    }

    /// Ids of living auto-attackers without a swing in flight, in id order.
    fn idle_attackers(&self) -> Vec<EntityId> {
        self.entities
            .values()
            .filter(|e| {
                !e.targeting.dead
                    && e.combat
                        .as_ref()
                        .is_some_and(|c| c.auto_attack && c.swing.is_none())
            })
            .map(|e| e.id)
            .collect()
    }

    /// Before movement: every idle auto-attacker out of reach heads for its target; one in
    /// reach stops chasing. A target that can no longer be fought ends the attack (a player
    /// also loses the selection).
    pub(super) fn chase(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        for id in self.idle_attackers() {
            let Some(attacker) = self.entities.get(&id) else {
                continue;
            };
            match self.engagement(tick, attacker) {
                None => self.drop_engagement(tick, id, events),
                Some(g) if g.in_reach => self.halt(tick, id, events),
                Some(g) => {
                    if let Some(e) = self.entities.get_mut(&id) {
                        e.dest = Some(g.pos);
                        if let Some(c) = e.combat.as_mut() {
                            c.chasing = true;
                        }
                    }
                },
            }
        }
    }

    /// The attack ended because its target cannot be fought: auto-attack off; a player's
    /// selection is cleared too.
    fn drop_engagement(&mut self, tick: Tick, id: EntityId, events: &mut Vec<ZoneEvent>) {
        self.disengage(tick, id, SwingCancel::TargetLost, events);
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        if e.kind == EntityKind::Player {
            if e.targeting.target.take().is_some() {
                events.push(ZoneEvent::TargetChanged {
                    tick,
                    entity: id,
                    target: None,
                });
            }
        } else {
            self.reselect(tick, id, events);
        }
    }

    /// Ends a chase where the entity stands.
    fn halt(&mut self, tick: Tick, id: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let Some(c) = e.combat.as_mut() else {
            return;
        };
        if c.chasing {
            c.chasing = false;
            if e.dest.take().is_some() {
                events.push(ZoneEvent::EntityMove {
                    tick,
                    entity: id,
                    pos: e.pos,
                    dest: None,
                    speed: e.speed,
                });
            }
        }
    }

    /// Lands every swing due on `tick`, attackers in id order. Each impact re-checks the
    /// attacker, the target's life and reach; a failed check cancels the swing without a
    /// draw. A lethal hit's consequences apply before the next attacker's impact, so a
    /// killed attacker's own later impact is cancelled.
    pub(super) fn land_impacts(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        let due: Vec<EntityId> = self
            .entities
            .values()
            .filter(|e| {
                e.combat
                    .as_ref()
                    .and_then(|c| c.swing)
                    .is_some_and(|s| s.impact == tick)
            })
            .map(|e| e.id)
            .collect();
        for id in due {
            self.land(tick, id, events);
        }
    }

    fn land(&mut self, tick: Tick, id: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(rules) = self.rules.clone() else {
            return;
        };
        let c = rules.constants();
        let Some(attacker) = self.entities.get(&id) else {
            return;
        };
        let Some(swing) = attacker
            .combat
            .as_ref()
            .and_then(|c| c.swing)
            .filter(|s| s.impact == tick)
        else {
            return; // cancelled earlier this tick
        };
        let check = self
            .engagement(tick, attacker)
            .filter(|g| g.target == swing.target && g.incarnation == swing.target_incarnation);
        let cancel = match check {
            None => Some(SwingCancel::TargetLost),
            Some(ref g) if !g.in_reach => Some(SwingCancel::OutOfRange),
            Some(_) => None,
        };
        if let Some(reason) = cancel {
            if let Some(ac) = self.entities.get_mut(&id).and_then(|e| e.combat.as_mut()) {
                ac.swing = None;
            }
            events.push(ZoneEvent::AttackCancelled {
                tick,
                attacker: id,
                target: swing.target,
                reason,
            });
            return;
        }
        let (Some(a_sheet), Some((t_sheet, t_hp, t_kind, t_level))) = (
            attacker.combat.as_ref().map(|c| c.sheet),
            self.entities.get(&swing.target).and_then(|t| {
                t.combat
                    .as_ref()
                    .map(|tc| (tc.sheet, tc.hp, t.kind, tc.sheet.level()))
            }),
        ) else {
            return;
        };
        let Some((outcome, damage)) = self.resolve(c, &a_sheet, &t_sheet) else {
            return;
        };
        let hp_after = t_hp.saturating_sub(damage);
        if let Some(ac) = self.entities.get_mut(&id).and_then(|e| e.combat.as_mut()) {
            ac.swing = None;
        }
        if let Some(tc) = self
            .entities
            .get_mut(&swing.target)
            .and_then(|e| e.combat.as_mut())
        {
            tc.hp = hp_after;
        }
        events.push(ZoneEvent::AttackResult {
            attacker: id,
            target: swing.target,
            tick,
            outcome,
            damage,
            target_hp_after: hp_after,
            target_incarnation: swing.target_incarnation,
        });
        if t_kind == EntityKind::Player {
            if let Some(tc) = self
                .entities
                .get(&swing.target)
                .and_then(|e| e.combat.as_ref())
            {
                events.push(stats_changed(tick, swing.target, tc));
            }
        }
        if hp_after == 0 {
            self.kill(tick, swing.target, Some(id), events);
        } else if t_kind == EntityKind::Npc {
            // HF: landed damage adds F(d*100/(L+7)); an attack worth 0 (a miss, or a scratch)
            // still adds 1 (SOURCES.md E-10, L2AttackableAI.onEvtAttacked).
            let hate = if damage > 0 {
                damage_hate(c, damage, t_level).unwrap_or(0)
            } else {
                0
            }
            .max(1);
            self.add_hate(
                tick,
                swing.target,
                id,
                HateEntry {
                    hate,
                    damage: damage.into(),
                },
                events,
            );
        }
    }

    /// The draws of a valid impact, in the fixed order: hit roll, then on a hit the crit
    /// roll and (radius > 0) the spread `j` in `-r..=r`. `None` only if the sheets are
    /// unusable, which their constructors rule out.
    fn resolve(
        &mut self,
        c: &FormulaConstants,
        attacker: &StatSheet,
        target: &StatSheet,
    ) -> Option<(AttackOutcome, u32)> {
        let chance = hit_chance_permille(c, attacker.accuracy(), target.evasion()).ok()?;
        let roll = self.roll_below(c.hit_roll_range);
        if !hit_lands(c, chance, roll).ok()? {
            return Some((AttackOutcome::Miss, 0));
        }
        let crit_roll = self.roll_below(c.crit_roll_range);
        let crit = crit_lands(c, attacker.crit_permille(), crit_roll).ok()?;
        let radius = attacker.random_damage();
        let spread = if radius == 0 {
            0
        } else {
            let width = radius.saturating_mul(2).saturating_add(1);
            i64::from(self.roll_below(width)).saturating_sub(i64::from(radius))
        };
        let damage = physical_damage(c, attacker, target, crit, spread).ok()?;
        Some((
            if crit {
                AttackOutcome::Crit
            } else {
                AttackOutcome::Hit
            },
            damage,
        ))
    }

    /// Death (plan E2.3; the consequences proper are E2.4's): HP 0, cycles cancelled, the
    /// victim's selection cleared, every attacker's target cleared, hate forgotten. Emits
    /// `EntityDied` exactly once per life.
    pub(super) fn kill(
        &mut self,
        tick: Tick,
        victim: EntityId,
        killer: Option<EntityId>,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(e) = self.entities.get(&victim) else {
            return;
        };
        if e.targeting.dead {
            return;
        }
        let incarnation = e.combat.as_ref().map_or(0, |c| c.incarnation);
        self.disengage(tick, victim, SwingCancel::AttackerDied, events);
        if let Some(e) = self.entities.get_mut(&victim) {
            e.targeting.dead = true;
            if let Some(c) = e.combat.as_mut() {
                c.hp = 0;
            }
            if e.dest.take().is_some() {
                events.push(ZoneEvent::EntityMove {
                    tick,
                    entity: victim,
                    pos: e.pos,
                    dest: None,
                    speed: e.speed,
                });
            }
            if e.targeting.target.take().is_some() {
                events.push(ZoneEvent::TargetChanged {
                    tick,
                    entity: victim,
                    target: None,
                });
            }
        }
        events.push(ZoneEvent::EntityDied {
            entity: victim,
            tick,
            killer,
            incarnation,
        });
        // E3.4 hook: corpse deadline and respawn schedule of a spawn-slot NPC (state_ai.rs).
        self.npc_died(tick, victim, events);
        if let Some(ledger) = self.hate.remove(&victim) {
            for (target, _) in ledger.iter() {
                events.push(ZoneEvent::HateChanged {
                    tick,
                    npc: victim,
                    target,
                    hate: 0,
                    damage: 0,
                });
            }
        }
        self.release_target(tick, victim, events);
    }

    /// After impacts: every idle auto-attacker whose cooldown is over and whose target is in
    /// reach stops and starts a swing; one out of reach keeps chasing.
    pub(super) fn start_swings(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        let Some(rules) = self.rules.clone() else {
            return;
        };
        for id in self.idle_attackers() {
            let Some(attacker) = self.entities.get(&id) else {
                continue;
            };
            let Some(g) = self.engagement(tick, attacker) else {
                self.drop_engagement(tick, id, events);
                continue;
            };
            if !g.in_reach {
                if let Some(e) = self.entities.get_mut(&id) {
                    e.dest = Some(g.pos);
                    if let Some(c) = e.combat.as_mut() {
                        c.chasing = true;
                    }
                }
                continue;
            }
            self.halt(tick, id, events);
            let Some(c) = self.entities.get_mut(&id).and_then(|e| e.combat.as_mut()) else {
                continue;
            };
            if tick < c.ready_at {
                continue;
            }
            let Ok(timing) = attack_timing(rules.constants(), c.sheet.attack_speed()) else {
                continue;
            };
            let swing = Swing {
                target: g.target,
                target_incarnation: g.incarnation,
                start: tick,
                impact: Tick(tick.0.saturating_add(timing.impact_ticks)),
                ready: Tick(tick.0.saturating_add(timing.cycle_ticks)),
            };
            c.swing = Some(swing);
            c.ready_at = swing.ready;
            events.push(ZoneEvent::AttackStarted {
                tick,
                attacker: id,
                target: g.target,
                target_incarnation: g.incarnation,
                impact: swing.impact,
                ready: swing.ready,
            });
        }
    }
}

/// Players fight attackable NPCs; NPCs fight players. Both must be living combatants.
pub(super) fn may_attack(attacker: &Entity, target: &Entity) -> bool {
    if attacker.id == target.id || target.targeting.dead || target.combat.is_none() {
        return false;
    }
    match (attacker.kind, target.kind) {
        (EntityKind::Player, EntityKind::Npc) => target.targeting.attackable,
        (EntityKind::Npc, EntityKind::Player) => {
            matches!(target.combat.as_ref().map(|c| &c.role), Some(CombatRole::Player { .. }))
        },
        (EntityKind::Player | EntityKind::Npc, EntityKind::Player | EntityKind::Npc) => false,
    }
}
