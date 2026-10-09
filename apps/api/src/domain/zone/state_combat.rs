//! The combat phases of [`ZoneState::run_tick`] and the combat commands (plan §3.2, Stories
//! E2.3–E2.6). Tick order: commands → spawn scheduler → NPC AI → chase → movement → due impacts by
//! attacker id (with their immediate damage, death, XP and level consequences) → start
//! eligible swings → progression facts → AOI output.
//!
//! Combat RNG draws at a valid impact: hit roll, then on a hit the crit roll and (radius > 0)
//! the damage spread. A lethal impact also schedules the NPC's next life, drawing respawn
//! jitter as documented in `state_ai.rs`. Cancelled or invalid swings, player death, XP and
//! player respawn draw nothing.
//!
//! Kill credit (E2.6): the NPC's full template XP goes once, on the death transition, to one
//! living player. Candidates are the player landing the killing blow and every other player
//! whose swing at the same life is due on the same tick (it is cancelled, undrawn, because
//! the target is dead). The candidate with the most recorded damage on the victim wins (the
//! hate ledger's damage, the killing blow included); ties go to the lowest `EntityId`.

use super::super::combat::{CombatRole, HateEntry, Swing, SwingCancel};
use super::super::combat_math::{
    attack_timing, crit_lands, damage_hate, hit_chance_permille, hit_lands, physical_damage,
    spawn_protection_ticks, town_respawn_vitals,
};
use super::super::command::{AttackOutcome, DeathFact, ProgressionDelta, RejectReason, ZoneEvent};
use super::super::entity::{Entity, EntityId, EntityKind, Tick};
use super::super::fixed::Vec2Fixed;
use super::super::progression::{add_xp, death_xp_loss, level_for_xp};
use super::super::stat_rules::FormulaConstants;
use super::super::stat_sheet::StatSheet;
use super::{stats_changed, ProgressNote, ZoneState};

/// The hit that killed: who landed it and for how much.
#[derive(Debug, Clone, Copy)]
pub(super) struct Blow {
    pub(super) attacker: EntityId,
    pub(super) damage: u32,
}

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
            self.kill(
                tick,
                swing.target,
                Some(Blow {
                    attacker: id,
                    damage,
                }),
                events,
            );
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

    /// Death (plan E2.3, E2.4): HP 0, cycles cancelled, the victim's selection cleared, every
    /// attacker's target cleared, hate forgotten, then the one-shot consequences: a player
    /// pays the HF death XP loss and may de-level; an NPC's XP is credited (see the module
    /// docs). Everything runs once per life: a second lethal hit
    /// finds the victim dead and does nothing.
    pub(super) fn kill(
        &mut self,
        tick: Tick,
        victim: EntityId,
        blow: Option<Blow>,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(e) = self.entities.get(&victim) else {
            return;
        };
        if e.targeting.dead {
            return;
        }
        let incarnation = e.combat.as_ref().map_or(0, |c| c.incarnation);
        let credit = self.kill_credit(tick, victim, blow);
        self.disengage(tick, victim, SwingCancel::AttackerDied, events);
        if let Some(e) = self.entities.get_mut(&victim) {
            e.targeting.dead = true;
            if let Some(c) = e.combat.as_mut() {
                c.hp = 0;
                c.protected_until = None;
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
        let killer = blow.map(|b| b.attacker);
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
        let role = self
            .entities
            .get(&victim)
            .and_then(|e| e.combat.as_ref())
            .map(|c| c.role.clone());
        match role {
            Some(CombatRole::Player { .. }) => self.charge_death(tick, victim, killer, events),
            Some(CombatRole::Npc { xp_reward, .. }) => {
                if let Some(player) = credit {
                    self.award_xp(tick, player, xp_reward, events);
                }
            },
            None => {},
        }
    }

    /// Who gets an NPC victim's XP (module docs): computed before the death clears the
    /// ledger and the same-tick swings. `None` for a player victim or without a player.
    fn kill_credit(&self, tick: Tick, victim: EntityId, blow: Option<Blow>) -> Option<EntityId> {
        let v = self.entities.get(&victim)?;
        let vc = v.combat.as_ref()?;
        if !matches!(vc.role, CombatRole::Npc { .. }) {
            return None;
        }
        let ledger = self.hate.get(&victim);
        let recorded = |id: EntityId| ledger.and_then(|l| l.get(id)).map_or(0, |r| r.damage);
        let is_player = |e: &Entity| {
            !e.targeting.dead
                && matches!(e.combat.as_ref().map(|c| &c.role), Some(CombatRole::Player { .. }))
        };
        let mut best: Option<(u64, EntityId)> = None;
        let mut consider = |id: EntityId, damage: u64| {
            // Ids arrive in ascending order after the blow, so `>` keeps the lowest on ties;
            // the blow is compared explicitly.
            let better = match best {
                None => true,
                Some((d, chosen)) => damage > d || (damage == d && id < chosen),
            };
            if better {
                best = Some((damage, id));
            }
        };
        if let Some(b) = blow {
            if self.entities.get(&b.attacker).is_some_and(is_player) {
                consider(b.attacker, recorded(b.attacker).saturating_add(b.damage.into()));
            }
        }
        for e in self.entities.values() {
            if Some(e.id) == blow.map(|b| b.attacker) || !is_player(e) {
                continue;
            }
            let due = e.combat.as_ref().and_then(|c| c.swing).is_some_and(|s| {
                s.target == victim && s.impact == tick && s.target_incarnation == vc.incarnation
            });
            if due {
                consider(e.id, recorded(e.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// A dead player pays the HF death loss once, `R((X[L+1]−X[L]) * loss[L])`, and drops to
    /// the level its XP now reaches; stats are recalculated and MP clamped.
    fn charge_death(
        &mut self,
        tick: Tick,
        player: EntityId,
        killer: Option<EntityId>,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(rules) = self.rules.clone() else {
            return;
        };
        let killer_template = killer
            .and_then(|k| self.entities.get(&k))
            .and_then(|k| k.combat.as_ref())
            .and_then(|c| match &c.role {
                CombatRole::Npc { template, .. } => Some(template.clone()),
                CombatRole::Player { .. } => None,
            });
        self.note_progress(player);
        let Some(c) = self
            .entities
            .get_mut(&player)
            .and_then(|e| e.combat.as_mut())
        else {
            return;
        };
        let level = c.sheet.level();
        let CombatRole::Player { xp, .. } = &mut c.role else {
            return;
        };
        let loss = death_xp_loss(&rules, level).unwrap_or(0).min(*xp);
        *xp = xp.saturating_sub(loss);
        if let Some(note) = self.progress.get_mut(&player) {
            note.died = Some(DeathFact {
                killer,
                killer_template,
                xp_lost: loss,
            });
        }
        self.relevel(player);
        if let Some(c) = self.entities.get(&player).and_then(|e| e.combat.as_ref()) {
            events.push(stats_changed(tick, player, c));
        }
    }

    /// Adds `reward` (capped at `X[86] − 1`) to a living player's XP, then emits `XpGained`,
    /// one `LevelUp` per threshold crossed and a final `StatsChanged` (plan §3.3).
    fn award_xp(&mut self, tick: Tick, player: EntityId, reward: u64, events: &mut Vec<ZoneEvent>) {
        let Some(rules) = self.rules.clone() else {
            return;
        };
        self.note_progress(player);
        let Some(c) = self
            .entities
            .get_mut(&player)
            .and_then(|e| e.combat.as_mut())
        else {
            return;
        };
        let CombatRole::Player { xp, .. } = &mut c.role else {
            return;
        };
        let before = *xp;
        *xp = add_xp(&rules, before, reward).unwrap_or(before);
        let (amount, total) = (xp.saturating_sub(before), *xp);
        if let Some(note) = self.progress.get_mut(&player) {
            note.xp_gained = note.xp_gained.saturating_add(amount);
        }
        events.push(ZoneEvent::XpGained {
            tick,
            entity: player,
            amount,
            total,
        });
        if let Some((old, new)) = self.relevel(player) {
            for level in old.saturating_add(1)..=new {
                events.push(ZoneEvent::LevelUp {
                    tick,
                    entity: player,
                    level,
                });
                if let Some(note) = self.progress.get_mut(&player) {
                    note.levels_gained.push(level);
                }
            }
            if let Some(c) = self.entities.get(&player).and_then(|e| e.combat.as_ref()) {
                events.push(stats_changed(tick, player, c));
            }
        }
    }

    /// Re-derives a player's level from its XP by threshold search; on a change rebuilds the
    /// stat sheet and clamps HP/MP to the new maxima (current values are kept otherwise).
    /// Returns `(old, new)` when the level changed.
    fn relevel(&mut self, player: EntityId) -> Option<(u32, u32)> {
        let rules = self.rules.clone()?;
        let c = self.entities.get_mut(&player)?.combat.as_mut()?;
        let CombatRole::Player { class, xp } = &c.role else {
            return None;
        };
        let (old, new) = (c.sheet.level(), level_for_xp(&rules, *xp));
        if old == new {
            return None;
        }
        let sheet =
            StatSheet::for_player(&rules, rules.class(class)?, new, Some(rules.starter_weapon()))
                .ok()?;
        c.sheet = sheet;
        c.hp = c.hp.min(sheet.max_hp());
        c.mp = c.mp.min(sheet.max_mp());
        Some((old, new))
    }

    /// `Respawn` (E2.4): a dead player returns to the safe point with
    /// `HP = max(1, F(maxHP * 65 / 100))`, MP 0, a new life and `PlayerSpawnProtection`
    /// (6000 ticks; an accepted `Attack` ends it early). No XP is refunded.
    pub(super) fn respawn_player(
        &mut self,
        tick: Tick,
        entity: EntityId,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let e = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if e.kind != EntityKind::Player {
            return Err(RejectReason::NotAPlayer);
        }
        let c = e.combat.as_ref().ok_or(RejectReason::NotPermitted)?;
        if !e.targeting.dead {
            return Err(RejectReason::NotDead);
        }
        let rules = self.rules.clone().ok_or(RejectReason::NotPermitted)?;
        let (hp, mp) = town_respawn_vitals(rules.constants(), &c.sheet)
            .map_err(|_| RejectReason::NotPermitted)?;
        let protection =
            spawn_protection_ticks(rules.constants()).map_err(|_| RejectReason::NotPermitted)?;
        let (from, to) = (e.pos, self.safe_point.unwrap_or(e.pos));
        self.note_progress(entity);
        if let Some(note) = self.progress.get_mut(&entity) {
            note.respawned = true;
        }
        let e = self
            .entities
            .get_mut(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        e.pos = to;
        e.dest = None;
        e.targeting.dead = false;
        let speed = e.speed;
        let c = e.combat.as_mut().ok_or(RejectReason::NotPermitted)?;
        c.hp = hp;
        c.mp = mp;
        c.incarnation = c.incarnation.saturating_add(1);
        c.protected_until = Some(Tick(tick.0.saturating_add(protection)));
        c.swing = None;
        c.auto_attack = false;
        c.chasing = false;
        let incarnation = c.incarnation;
        let stats = stats_changed(tick, entity, c);
        self.aoi.relocate(entity, from, to);
        let mut events = Vec::new();
        if from != to {
            events.push(ZoneEvent::EntityMove {
                tick,
                entity,
                pos: to,
                dest: None,
                speed,
            });
        }
        events.push(ZoneEvent::EntityRespawned {
            entity,
            tick,
            position: to,
            hp,
            incarnation,
        });
        events.push(stats);
        Ok(events)
    }

    /// Records a player's start-of-tick level and XP the first time its progression changes
    /// this tick.
    fn note_progress(&mut self, player: EntityId) {
        let Some((level, xp)) = self.entities.get(&player).and_then(|e| {
            let c = e.combat.as_ref()?;
            match c.role {
                CombatRole::Player { xp, .. } => Some((c.sheet.level(), xp)),
                CombatRole::Npc { .. } => None,
            }
        }) else {
            return;
        };
        self.progress.entry(player).or_insert_with(|| ProgressNote {
            level_before: level,
            xp_before: xp,
            xp_gained: 0,
            levels_gained: Vec::new(),
            died: None,
            respawned: false,
        });
    }

    /// End of tick: one `Progression` fact per player whose progression changed, in id
    /// order, with the end-of-tick values a checkpoint persists (E4.2).
    pub(super) fn flush_progression(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        for (entity, note) in std::mem::take(&mut self.progress) {
            let Some(e) = self.entities.get(&entity) else {
                continue;
            };
            let Some(c) = e.combat.as_ref() else {
                continue;
            };
            let CombatRole::Player { xp, .. } = c.role else {
                continue;
            };
            events.push(ZoneEvent::Progression(ProgressionDelta {
                tick,
                entity,
                level_before: note.level_before,
                xp_before: note.xp_before,
                level: c.sheet.level(),
                xp,
                hp: c.hp,
                mp: c.mp,
                alive: !e.targeting.dead,
                pos: e.pos,
                xp_gained: note.xp_gained,
                levels_gained: note.levels_gained,
                died: note.died,
                respawned: note.respawned,
            }));
        }
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
