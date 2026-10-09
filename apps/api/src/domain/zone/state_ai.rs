//! The NPC AI and spawn phases of [`ZoneState::run_tick`] (plan §3.2, Stories E3.2–E3.4).
//! Tick order: commands → **spawn phase** (corpse expiry, then due respawns) → **AI phase** →
//! chase → movement → impacts → **hit bookkeeping** → next swings → AOI output.
//!
//! RNG draws, all from the zone's `ChaCha12` stream via `roll_below` (unbiased rejection):
//! 1. Spawn phase, members in (slot, member) order: a member's *first* life draws its
//!    `EntityId` (16 bytes); respawns reuse it and draw nothing.
//! 2. AI phase, NPCs in id order, on their think tick (`tick % 10 == think_phase(id)`): an
//!    `Active` NPC that can move (speed > 0) and found no prey draws `roll_below(30)`; on 0 it
//!    wanders and draws `dx` then `dy`, each `roll_below(2R+1) − R` with R =
//!    [`MAX_DRIFT_RANGE`] milli-tiles.
//! 3. Death (inside the impact that kills): the respawn jitter `roll_below(random + 1)`, skipped
//!    when `random == 0`.
//!
//! Every intention change emits `ZoneEvent::NpcIntentionChanged`.

use super::super::ai::{
    Intention, MemberState, NpcAi, SlotMember, SpawnSlotSpec, MAX_ATTACK_TIMEOUT_TICKS,
    MAX_DRIFT_RANGE, RANDOM_WALK_RATE, THINK_INTERVAL_TICKS,
};
use super::super::combat::{HateEntry, SwingCancel};
use super::super::combat_math::npc_respawn_tick;
use super::super::command::{AttackOutcome, SessionGeneration, ZoneEvent};
use super::super::entity::{EntityId, EntityKind, Tick};
use super::super::fixed::{Fixed, Vec2Fixed};
use super::combat_phase::may_attack;
use super::{npc_combat, Spawned, ZoneState};

impl ZoneState {
    /// Installs the zone's spawn slots (E3.4). Every member's first life is due on the next
    /// tick; ignored for a zone without rules, which cannot hold combatants.
    #[must_use]
    pub fn with_spawn_slots(mut self, slots: Vec<SpawnSlotSpec>) -> Self {
        if self.rules.is_none() {
            return self;
        }
        self.members.clear();
        for (i, spec) in slots.iter().enumerate() {
            let Ok(slot) = u16::try_from(i) else { break };
            for member in 0..spec.count {
                let m = SlotMember { slot, member };
                self.members.insert(
                    m,
                    MemberState {
                        member: m,
                        entity: None,
                        incarnation: 0,
                        respawn_at: Some(self.next_tick),
                    },
                );
            }
        }
        self.slots = slots;
        self
    }

    /// The zone's spawn slots.
    #[must_use]
    pub fn spawn_slots(&self) -> &[SpawnSlotSpec] {
        &self.slots
    }

    /// The respawn scheduler's record of every slot member, in (slot, member) order.
    pub fn spawn_members(&self) -> impl Iterator<Item = &MemberState> {
        self.members.values()
    }

    /// Spawn phase: corpses past their deadline leave the world (NPC id order), then every
    /// member whose respawn is due gets a new life at home (slot, member order).
    pub(super) fn spawn_phase(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        let expired: Vec<EntityId> = self
            .entities
            .values()
            .filter(|e| {
                e.ai.as_ref()
                    .and_then(|ai| ai.corpse_until)
                    .is_some_and(|until| until <= tick)
            })
            .map(|e| e.id)
            .collect();
        for id in expired {
            events.extend(self.despawn(tick, id));
        }
        let due: Vec<SlotMember> = self
            .members
            .values()
            .filter(|m| m.respawn_at.is_some_and(|at| at <= tick))
            .map(|m| m.member)
            .collect();
        for m in due {
            self.spawn_member(tick, m, events);
        }
    }

    /// A new life for `m`: same entity id, next incarnation, full stats, `Idle` at home.
    /// A member whose entity is still in the world is never spawned twice.
    fn spawn_member(&mut self, tick: Tick, m: SlotMember, events: &mut Vec<ZoneEvent>) {
        let Some(state) = self.members.get(&m).copied() else {
            return;
        };
        let Some(spec) = self.slots.get(usize::from(m.slot)).cloned() else {
            return;
        };
        let id = match state.entity {
            Some(id) => id,
            None => self.random_entity_id(),
        };
        if self.entities.contains_key(&id) {
            if let Some(s) = self.members.get_mut(&m) {
                s.respawn_at = None;
            }
            return;
        }
        let incarnation = state.incarnation.saturating_add(1);
        let Ok(mut combat) = npc_combat(&spec.combat) else {
            return;
        };
        combat.incarnation = incarnation;
        let hp = combat.hp;
        let Ok(spawned) = self.spawn(
            tick,
            Spawned {
                id,
                kind: EntityKind::Npc,
                name: &spec.name,
                pos: spec.home,
                speed: spec.speed,
                generation: SessionGeneration::default(),
                combat: Some(combat),
            },
        ) else {
            return;
        };
        if let Some(e) = self.entities.get_mut(&id) {
            e.ai = Some(NpcAi {
                slot: m,
                home: spec.home,
                intention: Intention::Idle,
                last_hit: tick,
                called_help: false,
                corpse_until: None,
            });
        }
        self.members.insert(
            m,
            MemberState {
                member: m,
                entity: Some(id),
                incarnation,
                respawn_at: None,
            },
        );
        events.extend(spawned);
        if incarnation > 1 {
            events.push(ZoneEvent::EntityRespawned {
                entity: id,
                tick,
                position: spec.home,
                hp,
            });
            events.push(ZoneEvent::NpcIntentionChanged {
                tick,
                entity: id,
                from: Intention::Dead,
                to: Intention::Idle,
            });
        }
    }

    /// AI phase: every living AI NPC in id order. `Attack` checks target, leash and timeout
    /// and `ReturnHome` checks arrival on every tick; the rest happens on the NPC's think tick.
    pub(super) fn ai_phase(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        let npcs: Vec<EntityId> = self
            .entities
            .values()
            .filter(|e| e.ai.is_some() && !e.targeting.dead)
            .map(|e| e.id)
            .collect();
        for id in npcs {
            match self.intention(id) {
                Some(Intention::Attack) => self.check_attack(tick, id, events),
                Some(Intention::ReturnHome) => self.check_return(tick, id, events),
                Some(Intention::Idle | Intention::Active | Intention::Dead) | None => {},
            }
            if tick.0 % THINK_INTERVAL_TICKS == NpcAi::think_phase(id) {
                self.think(tick, id, events);
            }
        }
    }

    /// After impacts: an AI NPC that landed a hit (not a miss) this tick resets its attack
    /// timeout. `events[from..]` are this tick's impact events.
    pub(super) fn note_hits(&mut self, tick: Tick, events: &[ZoneEvent]) {
        for event in events {
            let ZoneEvent::AttackResult {
                attacker, outcome, ..
            } = event
            else {
                continue;
            };
            if *outcome == AttackOutcome::Miss {
                continue;
            }
            if let Some(ai) = self.entities.get_mut(attacker).and_then(|e| e.ai.as_mut()) {
                if ai.intention == Intention::Attack {
                    ai.last_hit = tick;
                }
            }
        }
    }

    /// Hook at the end of `reselect`: an idle or active NPC that acquired a target attacks;
    /// one in `Attack` left with nobody re-evaluates to `ReturnHome`.
    pub(super) fn ai_target_changed(
        &mut self,
        tick: Tick,
        npc: EntityId,
        chosen: Option<EntityId>,
        events: &mut Vec<ZoneEvent>,
    ) {
        match (self.intention(npc), chosen) {
            (Some(Intention::Idle | Intention::Active), Some(_)) => {
                self.enter_attack(tick, npc, events);
            },
            (Some(Intention::Attack), None) => self.return_home(tick, npc, events),
            _ => {},
        }
    }

    /// Hook in `kill` (the E2.4 death consequences): an AI NPC becomes a corpse until
    /// `corpse_decay_ticks` pass and its slot schedules the next life at
    /// `deathTick + 10*(delay + jitter)`, never before the corpse is gone. The jitter is drawn
    /// here, once per death.
    pub(super) fn npc_died(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(slot) = self
            .entities
            .get(&npc)
            .and_then(|e| e.ai.as_ref())
            .map(|ai| ai.slot)
        else {
            return;
        };
        let Some(spec) = self.slots.get(usize::from(slot.slot)) else {
            return;
        };
        let (corpse_ticks, delay, random) =
            (spec.brain.corpse_decay_ticks, spec.respawn_delay_secs, spec.respawn_random_secs);
        self.set_intention(tick, npc, Intention::Dead, events);
        let corpse_until = Tick(tick.0.saturating_add(corpse_ticks.into()));
        if let Some(ai) = self.entities.get_mut(&npc).and_then(|e| e.ai.as_mut()) {
            ai.corpse_until = Some(corpse_until);
            ai.called_help = false;
        }
        let jitter = if random == 0 {
            0
        } else {
            self.roll_below(random.saturating_add(1))
        };
        let respawn = npc_respawn_tick(tick, delay, random, jitter)
            .unwrap_or(Tick(u64::MAX))
            .max(corpse_until.next());
        if let Some(m) = self.members.get_mut(&slot) {
            m.respawn_at = Some(respawn);
        }
    }

    /// `true` when `npc` is walking home, so it may not be aggroed (plan §3.2).
    pub(super) fn is_returning(&self, npc: EntityId) -> bool {
        self.intention(npc) == Some(Intention::ReturnHome)
    }

    fn intention(&self, npc: EntityId) -> Option<Intention> {
        self.entities
            .get(&npc)
            .and_then(|e| e.ai.as_ref())
            .map(|ai| ai.intention)
    }

    fn set_intention(
        &mut self,
        tick: Tick,
        npc: EntityId,
        to: Intention,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(ai) = self.entities.get_mut(&npc).and_then(|e| e.ai.as_mut()) else {
            return;
        };
        let from = ai.intention;
        if from == to {
            return;
        }
        ai.intention = to;
        events.push(ZoneEvent::NpcIntentionChanged {
            tick,
            entity: npc,
            from,
            to,
        });
    }

    /// The slot spec of an AI NPC.
    fn spec_of(&self, npc: EntityId) -> Option<&SpawnSlotSpec> {
        let ai = self.entities.get(&npc)?.ai.as_ref()?;
        self.slots.get(usize::from(ai.slot.slot))
    }

    /// Every tick in `Attack`: no target, beyond the leash or [`MAX_ATTACK_TIMEOUT_TICKS`]
    /// without a landed hit → `ReturnHome`.
    fn check_attack(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        let (Some(e), Some(spec)) = (self.entities.get(&npc), self.spec_of(npc)) else {
            return;
        };
        let Some(ai) = e.ai.as_ref() else { return };
        let leashed = !e.pos.within(ai.home, spec.brain.leash_radius);
        let timed_out = tick.0.saturating_sub(ai.last_hit.0) >= MAX_ATTACK_TIMEOUT_TICKS;
        if e.targeting.target.is_none() || leashed || timed_out {
            self.return_home(tick, npc, events);
        }
    }

    /// Every tick in `ReturnHome`: at home → full HP, attackable, `Active`; otherwise keep
    /// heading home.
    fn check_return(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(e) = self.entities.get_mut(&npc) else {
            return;
        };
        let Some(home) = e.ai.as_ref().map(|ai| ai.home) else {
            return;
        };
        if e.pos != home {
            if e.dest != Some(home) {
                e.dest = Some(home);
            }
            return;
        }
        e.dest = None;
        e.targeting.attackable = true;
        if let Some(c) = e.combat.as_mut() {
            c.hp = c.sheet.max_hp();
            c.mp = c.sheet.max_mp();
        }
        self.set_intention(tick, npc, Intention::Active, events);
    }

    /// The think proper (every [`THINK_INTERVAL_TICKS`]).
    fn think(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(pos) = self.entities.get(&npc).map(|e| e.pos) else {
            return;
        };
        match self.intention(npc) {
            Some(Intention::Idle) => {
                if self.players_nearby(pos) {
                    self.set_intention(tick, npc, Intention::Active, events);
                    self.think_active(tick, npc, events);
                }
            },
            Some(Intention::Active) => {
                if self.players_nearby(pos) {
                    self.think_active(tick, npc, events);
                } else {
                    self.set_intention(tick, npc, Intention::Idle, events);
                }
            },
            // Re-evaluate: the most hated eligible target, ties keeping the current one.
            Some(Intention::Attack) => self.reselect(tick, npc, events),
            Some(Intention::ReturnHome | Intention::Dead) | None => {},
        }
    }

    /// `Active`: an aggressive NPC aggroes the nearest eligible player in range (1 hate; equal
    /// distance → lowest id); otherwise a mobile NPC wanders on 1 think in 30.
    fn think_active(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(spec) = self.spec_of(npc) else {
            return;
        };
        let (aggressive, aggro_range) = (spec.brain.aggressive, spec.brain.aggro_range);
        if aggressive {
            if let Some(prey) = self.nearest_prey(tick, npc, aggro_range) {
                self.add_hate(tick, npc, prey, HateEntry { hate: 1, damage: 0 }, events);
                return;
            }
        }
        let Some(e) = self.entities.get(&npc) else {
            return;
        };
        if e.speed.milli_tiles_per_tick() == 0 {
            return;
        }
        if self.roll_below(RANDOM_WALK_RATE) != 0 {
            return;
        }
        let r = MAX_DRIFT_RANGE.raw();
        let width = u32::try_from(r)
            .unwrap_or(0)
            .saturating_mul(2)
            .saturating_add(1);
        let dx = i32::try_from(self.roll_below(width))
            .unwrap_or(0)
            .saturating_sub(r);
        let dy = i32::try_from(self.roll_below(width))
            .unwrap_or(0)
            .saturating_sub(r);
        let Some(e) = self.entities.get_mut(&npc) else {
            return;
        };
        let Some(home) = e.ai.as_ref().map(|ai| ai.home) else {
            return;
        };
        let dest = clamp_to(
            self.bounds.min(),
            self.bounds.max(),
            Vec2Fixed::new(
                home.x.saturating_add(Fixed::from_raw(dx)),
                home.y.saturating_add(Fixed::from_raw(dy)),
            ),
        );
        if dest != e.pos {
            e.dest = Some(dest);
        }
    }

    /// Any living player in the NPC's AOI (L2J: an active region).
    fn players_nearby(&self, pos: Vec2Fixed) -> bool {
        self.aoi.in_aoi(pos).any(|id| {
            self.entities
                .get(&id)
                .is_some_and(|e| e.kind == EntityKind::Player && !e.targeting.dead)
        })
    }

    /// The nearest player `npc` may attack within `range` (squared distance, then lowest id).
    fn nearest_prey(&self, tick: Tick, npc: EntityId, range: Fixed) -> Option<EntityId> {
        let me = self.entities.get(&npc)?;
        self.aoi
            .in_aoi(me.pos)
            .filter_map(|id| self.entities.get(&id))
            .filter(|p| {
                p.kind == EntityKind::Player
                    && may_attack(me, p)
                    && !p.combat.as_ref().is_some_and(|c| c.protected_at(tick))
                    && me.pos.within(p.pos, range)
            })
            .map(|p| (me.pos.distance_sq(p.pos), p.id))
            .min()
            .map(|(_, id)| id)
    }

    /// Into `Attack`: stop wandering, start the timeout, and, unless this engagement already
    /// made or answered one, call the clan.
    fn enter_attack(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        self.set_intention(tick, npc, Intention::Attack, events);
        let Some(e) = self.entities.get_mut(&npc) else {
            return;
        };
        let chasing = e.combat.as_ref().is_some_and(|c| c.chasing);
        if !chasing && e.dest.take().is_some() {
            events.push(ZoneEvent::EntityMove {
                tick,
                entity: npc,
                pos: e.pos,
                dest: None,
                speed: e.speed,
            });
        }
        let Some(ai) = e.ai.as_mut() else { return };
        ai.last_hit = tick;
        if !ai.called_help {
            ai.called_help = true;
            self.clan_call(tick, npc, events);
        }
    }

    /// Social aggro: every idle or active NPC of the caller's clan within its
    /// `clan_help_range`, in id order, adds 1 hate for the caller's target. Helpers are marked
    /// as having answered, so they never call in turn.
    fn clan_call(&mut self, tick: Tick, caller: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(me) = self.entities.get(&caller) else {
            return;
        };
        let Some(target_id) = me.targeting.target else {
            return;
        };
        let Some(spec) = self.spec_of(caller) else {
            return;
        };
        let Some(clan) = spec.brain.clan.clone() else {
            return;
        };
        let range = spec.brain.clan_help_range;
        let Some(target) = self.entities.get(&target_id) else {
            return;
        };
        let allies: Vec<EntityId> = self
            .entities
            .values()
            .filter(|a| {
                a.id != caller
                    && !a.targeting.dead
                    && a.ai.as_ref().is_some_and(|ai| {
                        matches!(ai.intention, Intention::Idle | Intention::Active)
                            && self
                                .slots
                                .get(usize::from(ai.slot.slot))
                                .is_some_and(|s| s.brain.clan.as_deref() == Some(clan.as_str()))
                    })
                    && a.pos.within(me.pos, range)
                    && may_attack(a, target)
            })
            .map(|a| a.id)
            .collect();
        for ally in allies {
            if let Some(ai) = self.entities.get_mut(&ally).and_then(|e| e.ai.as_mut()) {
                ai.called_help = true;
            }
            self.add_hate(tick, ally, target_id, HateEntry { hate: 1, damage: 0 }, events);
        }
    }

    /// Into `ReturnHome`: cancel the attack, drop the target and the whole hate list, become
    /// unattackable and walk home (plan §3.2).
    fn return_home(&mut self, tick: Tick, npc: EntityId, events: &mut Vec<ZoneEvent>) {
        self.set_intention(tick, npc, Intention::ReturnHome, events);
        self.disengage(tick, npc, SwingCancel::TargetLost, events);
        if let Some(ledger) = self.hate.remove(&npc) {
            for (target, _) in ledger.iter() {
                events.push(ZoneEvent::HateChanged {
                    tick,
                    npc,
                    target,
                    hate: 0,
                    damage: 0,
                });
            }
        }
        let Some(e) = self.entities.get_mut(&npc) else {
            return;
        };
        if e.targeting.target.take().is_some() {
            events.push(ZoneEvent::TargetChanged {
                tick,
                entity: npc,
                target: None,
            });
        }
        e.targeting.attackable = false;
        let Some(ai) = e.ai.as_mut() else { return };
        ai.called_help = false;
        let home = ai.home;
        e.dest = (e.pos != home).then_some(home);
    }
}

/// `p` clamped into the inclusive rectangle `min..=max`.
fn clamp_to(min: Vec2Fixed, max: Vec2Fixed, p: Vec2Fixed) -> Vec2Fixed {
    Vec2Fixed::new(p.x.max(min.x).min(max.x), p.y.max(min.y).min(max.y))
}
