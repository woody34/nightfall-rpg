//! Complete recorded offline cores use the same critical writer without a live session.
use super::*;
use crate::domain::zone::{ItemPausedCore, Phase4Projection, ZoneEvent};

impl CheckpointService {
    pub(super) async fn paused_items(
        &mut self,
        tick: &AppliedTick,
        snapshot: &ZoneSnapshot,
    ) -> anyhow::Result<()> {
        let mut owners = std::collections::BTreeSet::new();
        for event in &tick.events {
            let ZoneEvent::ItemCheckpoint(projection) = event else {
                continue;
            };
            anyhow::ensure!(self.item_repo.is_some(), "item checkpoint repository missing");
            anyhow::ensure!(
                owners.insert(projection.entity),
                "multiple final item checkpoints for one tick/owner"
            );
            if snapshot
                .entities
                .iter()
                .any(|entity| entity.id == projection.entity)
            {
                anyhow::ensure!(
                    projection.paused_core.is_none(),
                    "live item projection carries offline core"
                );
                continue;
            }
            let core = projection
                .paused_core
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("offline item projection missing final core"))?;
            let original = tick
                .commands
                .iter()
                .find_map(|command| {
                    if tick
                        .dispositions
                        .iter()
                        .any(|d| d.ordinal == command.ordinal)
                    {
                        return None;
                    }
                    if let ZoneCommand::MaterializePendingItems(grant) = &command.command {
                        if grant.entity == projection.entity && grant.account == projection.account
                        {
                            return grant.paused.as_deref();
                        }
                    }
                    None
                })
                .ok_or_else(|| anyhow::anyhow!("offline item checkpoint has no recorded input"))?;
            anyhow::ensure!(
                original.load.checkpoint_revision == core.load.checkpoint_revision,
                "offline item revision fence changed"
            );
            anyhow::ensure!(
                !self.players.contains_key(&projection.entity),
                "offline item owner has an existing lane"
            );
            let cp = paused_checkpoint(core, projection, snapshot.seed.zone, tick.epoch)?;
            let work = super::super::ports::ItemCheckpointWork {
                zone: snapshot.seed.zone,
                epoch: tick.epoch,
                allocation: snapshot
                    .phase4
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("item allocation missing"))?
                    .allocator,
                projection: projection.as_ref().clone(),
            };
            anyhow::ensure!(projection.tick == tick.tick, "offline item projection tick mismatch");
            work.validate(&cp, projection.account)?;
            let events = item_events(&cp, &work, 0)?;
            self.players.insert(
                projection.entity,
                Player {
                    revision: cp.revision_seen,
                    last_saved: tick.tick,
                    latest: Some(cp),
                    last_phase3: None,
                    events,
                    learning: None,
                    items: Some(work),
                    dirty: true,
                    fenced: false,
                },
            );
            // Keep the exact request in the lane until acknowledged, including on failure.
            self.flush(projection.entity).await?;
            self.players.remove(&projection.entity);
        }
        Ok(())
    }

    /// Settle pending critical facts and item work. At the end of historical recovery
    /// (`all_retained`) every retained Phase 3 lane is flushed, since the internal-state
    /// cadence leaves such state persisted only on cadence (Phase 3 §10 P2).
    pub(super) async fn recover_pending(&mut self, all_retained: bool) -> anyhow::Result<()> {
        let pending: Vec<_> = self
            .players
            .iter()
            .filter(|(_, p)| {
                let retained = p.latest.as_ref().is_some_and(|cp| cp.phase3.is_some());
                !p.events.is_empty() || p.items.is_some() || (retained && (all_retained || p.dirty))
            })
            .map(|(id, _)| *id)
            .collect();
        for entity in pending {
            let offline = self.players.get(&entity).is_some_and(|p| {
                p.items
                    .as_ref()
                    .is_some_and(|work| work.projection.paused_core.is_some())
            });
            self.flush(entity).await?;
            // An absent owner has no periodic/session lifecycle after its private write.
            if offline {
                self.players.remove(&entity);
            }
        }
        Ok(())
    }
}

pub(crate) fn paused_checkpoint(
    core: &ItemPausedCore,
    projection: &Phase4Projection,
    zone: ZoneId,
    epoch: u64,
) -> anyhow::Result<CharacterCheckpoint> {
    core.validate(projection.account)?;
    let load = &core.load;
    let progression = load
        .progression
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("offline progression missing"))?;
    Ok(CharacterCheckpoint {
        character_id: CharacterId::from_uuid(projection.entity.as_uuid()),
        revision_seen: load
            .checkpoint_revision
            .ok_or_else(|| anyhow::anyhow!("offline revision missing"))?,
        level: load.level,
        xp: load.xp,
        hp: load
            .hp
            .ok_or_else(|| anyhow::anyhow!("offline HP missing"))?,
        mp: load
            .mp
            .ok_or_else(|| anyhow::anyhow!("offline MP missing"))?,
        alive: load.alive,
        position: position(core.position),
        class_state: Some(progression.class_state.clone()),
        phase3: load.phase3.clone(),
        phase4: Some(projection.checkpoint.clone()),
        idempotency: (
            "save_checkpoint".into(),
            IdempotencyKey::from_uuid(stable_id(
                projection.entity,
                zone,
                epoch,
                projection.tick,
                None,
            )),
        ),
    })
}
