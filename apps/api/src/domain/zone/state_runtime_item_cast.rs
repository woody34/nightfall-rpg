//! Both cast lanes share deadlines, target filtering, resource payment and handlers.
use super::{
    missing, CastCancelReason, CastId, CastState, EntityId, TargetBinding, Tick, TickError,
    ZoneEvent, ZoneState,
};
use crate::domain::zone::phase4::ItemActionBinding;
use crate::domain::zone::state_effects::CastWork;
use crate::domain::zone::CastInFlight;

impl ZoneState {
    pub(super) fn cast_work_v6(
        &self,
        tick: Tick,
        launch: bool,
    ) -> Result<Vec<CastWork>, TickError> {
        let phase = self.phase3.as_ref().ok_or_else(missing)?;
        let mut work = if launch {
            phase.schedule.cast_launches(tick)
        } else {
            phase.schedule.cast_impacts(tick)
        };
        if let Some(items) = &self.phase4 {
            for lane in items
                .canonical
                .item_casts
                .values()
                .filter(|lane| lane.simultaneous)
            {
                let at = if launch {
                    lane.cast.launch_at
                } else {
                    lane.cast.complete_at
                };
                if at <= tick && lane.cast.launched != launch {
                    work.push(CastWork {
                        at,
                        ordinal: self.sidecar(lane.caster.entity)?.ordinal,
                        caster: lane.caster.entity,
                        cast: lane.cast.id,
                    });
                }
            }
            work.sort_unstable();
        }
        Ok(work)
    }

    pub(super) fn cast_for_work_v6(
        &self,
        work: CastWork,
    ) -> Result<Option<CastInFlight>, TickError> {
        if let CastState::Casting(cast) = &self.sidecar(work.caster)?.cast {
            if cast.id == work.cast {
                return Ok(Some(cast.clone()));
            }
        }
        Ok(self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.item_casts.get(&work.cast))
            .filter(|lane| lane.simultaneous && lane.caster.entity == work.caster)
            .map(|lane| lane.cast.clone()))
    }

    pub(super) fn launch_cast_work_v6(
        &mut self,
        tick: Tick,
        work: CastWork,
        targets: &[TargetBinding],
    ) -> Result<(), TickError> {
        if matches!(&self.sidecar(work.caster)?.cast, CastState::Casting(cast) if cast.id == work.cast)
        {
            self.sidecar_mut(work.caster)?
                .launch_cast(tick, work.cast, |target| targets.contains(&target));
            let CastState::Casting(cast) = self.sidecar(work.caster)?.cast.clone() else {
                return Err(missing());
            };
            if let Some(lane) = self
                .phase4
                .as_mut()
                .and_then(|items| items.canonical.item_casts.get_mut(&work.cast))
            {
                lane.cast = cast;
            }
        } else if let Some(lane) = self
            .phase4
            .as_mut()
            .and_then(|items| items.canonical.item_casts.get_mut(&work.cast))
        {
            if !lane.simultaneous || lane.caster.entity != work.caster {
                return Err(missing());
            }
            lane.cast.launch(tick, |target| targets.contains(&target));
        }
        Ok(())
    }

    pub(super) fn pay_cast_work_v6(
        &mut self,
        tick: Tick,
        work: CastWork,
        mp: u32,
        hp: u32,
    ) -> Result<Result<Option<CastInFlight>, CastCancelReason>, TickError> {
        let Some(cast) = self.cast_for_work_v6(work)? else {
            return Ok(Ok(None));
        };
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .schedule_dirty
            .insert(work.caster);
        let hp_remainder_q = if self.phase4.is_some() {
            self.sidecar(work.caster)?.fractions.hp
        } else {
            0
        };
        let combat = self
            .entities
            .get_mut(&work.caster)
            .and_then(|entity| entity.combat.as_mut())
            .ok_or_else(missing)?;
        let result = cast.pay_impact(tick, &mut combat.mp, &mut combat.hp, mp, hp, hp_remainder_q);
        if result.is_err() {
            self.finish_cast_work_v6(work)?;
        }
        Ok(result)
    }

    pub(super) fn finish_cast_work_v6(&mut self, work: CastWork) -> Result<(), TickError> {
        self.sidecar_mut(work.caster)?.finish_cast(work.cast);
        self.remove_item_cast_context(work.cast);
        Ok(())
    }

    fn remove_item_cast_context(&mut self, cast: CastId) {
        if let Some(items) = &mut self.phase4 {
            items.canonical.item_casts.remove(&cast);
            items
                .canonical
                .charge_bindings
                .retain(|binding| binding.action != (ItemActionBinding::Cast { id: cast }));
        }
    }

    pub(super) fn cancel_item_casts_v6(
        &mut self,
        tick: Tick,
        entity: EntityId,
        id: Option<CastId>,
        reason: CastCancelReason,
        events: &mut Vec<ZoneEvent>,
    ) {
        let removed: Vec<_> = self
            .phase4
            .as_ref()
            .into_iter()
            .flat_map(|items| items.canonical.item_casts.values())
            .filter(|lane| lane.caster.entity == entity && id.is_none_or(|id| id == lane.cast.id))
            .cloned()
            .collect();
        for lane in removed {
            self.remove_item_cast_context(lane.cast.id);
            if lane.simultaneous {
                events.push(ZoneEvent::SkillCastCancelled {
                    tick,
                    caster: lane.caster,
                    cast_id: lane.cast.id,
                    reason,
                });
            }
        }
    }
}
