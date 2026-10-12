//! Direct grants share inventory/fold/receipt rules; paused owners never enter the world.
use super::*;
use crate::domain::zone::item_inventory::{
    InventoryBoundary, InventoryEdit, InventoryLimits, InventoryMutation,
};
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemDeltaPlan, ItemOperation, ItemReceiptFact, ItemReceiptRef,
    ItemResultDetail,
};
use crate::domain::zone::state_skill::ItemReceiptState;
use crate::domain::zone::{ItemGrantCommand, ItemPausedCore, Phase4PlayerLoad, Phase4Projection};

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

impl ZoneState {
    #[allow(clippy::too_many_lines)] // Preflight precedes both live and private candidate execution.
    pub(super) fn grant_item_v6(
        &mut self,
        tick: Tick,
        command: &ItemGrantCommand,
        events: &mut Vec<ZoneEvent>,
        receipts: &mut BTreeMap<(crate::domain::AccountId, ItemReceiptRef), ItemReceiptState>,
    ) -> Result<(), RejectReason> {
        command.validate().map_err(invalid)?;
        if self
            .phase4
            .as_ref()
            .is_none_or(|items| items.rules.hash() != command.rules_hash)
        {
            return Err(RejectReason::InvalidLoad);
        }
        let reference = ItemReceiptRef {
            operation: ItemOperation::Grant,
            key: command.request_id,
        };
        let scope = (command.account, reference);
        let recorded = command.known.as_ref().map(|known| known.result.clone());
        if let Some(stored) = &command.conflict {
            if receipts.get(&scope).is_some_and(|old| match old {
                ItemReceiptState::Known(old) => &old.fingerprint != stored,
                ItemReceiptState::Conflict(old) => old != stored,
            }) {
                return Err(RejectReason::InvalidLoad);
            }
            receipts
                .entry(scope)
                .or_insert_with(|| ItemReceiptState::Conflict(stored.clone()));
            return Err(RejectReason::ItemOperationConflict);
        }
        if let Some(old) = receipts.get(&scope) {
            match old {
                ItemReceiptState::Known(old) => {
                    if old.fingerprint != command.fingerprint
                        || recorded.as_ref().is_some_and(|r| r != &old.result)
                    {
                        return Err(RejectReason::ItemOperationConflict);
                    }
                    return Ok(());
                },
                ItemReceiptState::Conflict(old)
                    if old != &command.fingerprint || recorded.is_none() =>
                {
                    return Err(RejectReason::ItemOperationConflict);
                },
                ItemReceiptState::Conflict(_) => {},
            }
        }
        if let Some(result) = recorded {
            receipts.insert(
                scope,
                ItemReceiptState::Known(ItemReceiptFact {
                    request_id: command.request_id,
                    fingerprint: command.fingerprint.clone(),
                    result,
                }),
            );
            return Ok(());
        }
        let previous = events.iter().find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(p) = event {
                if p.entity == command.entity {
                    return Some(p.as_ref().clone());
                }
            }
            None
        });
        if previous.as_ref().is_some_and(|p| {
            p.account != command.account || p.paused_core.is_some() != command.paused.is_some()
        }) {
            return Err(RejectReason::InvalidLoad);
        }
        let mut candidate = self.clone();
        let mut previous = previous;
        let core = if let Some(input) = &command.paused {
            if self.entities.contains_key(&command.entity) {
                return Err(RejectReason::AlreadyExists);
            }
            let core = previous
                .as_ref()
                .and_then(|p| p.paused_core.as_deref())
                .unwrap_or(input);
            if core.load.checkpoint_revision != input.load.checkpoint_revision {
                return Err(RejectReason::InvalidLoad);
            }
            candidate.attach_paused_items_v6(tick, command.entity, core)?;
            let mut kit = Vec::new();
            candidate
                .materialize_kit_v6(tick, command.entity, Some(&core.load), &mut kit)
                .map_err(invalid)?;
            if let Some(ZoneEvent::ItemCheckpoint(projection)) = kit.pop() {
                if previous.is_some() {
                    return Err(RejectReason::InvalidLoad);
                }
                previous = Some(*projection);
            }
            Some(input.as_ref())
        } else {
            None
        };
        let mut projection = candidate.apply_grant_v6(tick, command, previous)?;
        if let Some(core) = core {
            projection.paused_core =
                Some(Box::new(candidate.freeze_paused_items_v6(tick, command.entity, core)?));
            for receipt in &mut projection.receipts {
                receipt.result.incarnation = 0;
            }
            projection
                .validate_structure(
                    projection
                        .paused_core
                        .as_ref()
                        .and_then(|core| core.load.phase3.as_ref())
                        .ok_or(RejectReason::InvalidLoad)?
                        .modifier_install_order
                        .len(),
                )
                .map_err(invalid)?;
            // Private restoration allocates temporary provider/effect ordinals only. Neither
            // those nor its caches, schedules, entities or observer state enter the zone.
            self.phase4
                .as_mut()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .allocator = candidate
                .phase4
                .as_ref()
                .ok_or(RejectReason::InvalidLoad)?
                .canonical
                .allocator;
        } else {
            *self = candidate;
        }
        let receipt = projection
            .receipts
            .iter()
            .find(|r| r.reference() == reference)
            .ok_or(RejectReason::InvalidLoad)?
            .clone();
        receipts.insert(scope, ItemReceiptState::Known(receipt));
        events.retain(
            |event| !matches!(event, ZoneEvent::ItemCheckpoint(p) if p.entity == command.entity),
        );
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        Ok(())
    }

    fn attach_paused_items_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        core: &ItemPausedCore,
    ) -> Result<(), RejectReason> {
        self.validate_paused_load_v5(Some(&core.load))?;
        let combat = self.loaded_player(
            self.rules.as_deref().ok_or(RejectReason::InvalidLoad)?,
            &core.load,
            id,
        )?;
        self.entities.insert(
            id,
            Entity {
                id,
                kind: EntityKind::Player,
                name: core.name.clone(),
                pos: core.position,
                dest: None,
                speed: Speed::from_milli_tiles_per_tick(0),
                generation: SessionGeneration::default(),
                targeting: crate::domain::zone::TargetingState {
                    dead: combat.hp == 0,
                    ..Default::default()
                },
                combat: Some(combat),
                ai: None,
            },
        );
        self.attach_v5(tick, id, Some(&core.load)).map_err(invalid)
    }

    fn freeze_paused_items_v6(
        &self,
        tick: Tick,
        id: EntityId,
        before: &ItemPausedCore,
    ) -> Result<ItemPausedCore, RejectReason> {
        let combat = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or(RejectReason::InvalidLoad)?;
        let CombatRole::Player {
            progression,
            xp,
            class,
        } = &combat.role
        else {
            return Err(RejectReason::NotAPlayer);
        };
        let phase = &self
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        let items = &self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        Ok(ItemPausedCore {
            name: before.name.clone(),
            position: before.position,
            load: PlayerLoad {
                phase4: Some(Phase4PlayerLoad {
                    checkpoint: Some(items.checkpoint(id, tick, phase).map_err(invalid)?),
                    pending: Vec::new(),
                }),
                phase3: Some(phase.checkpoint(id, tick).map_err(invalid)?),
                progression: progression.clone(),
                checkpoint_revision: before.load.checkpoint_revision,
                class: class.clone(),
                level: combat.sheet.level(),
                xp: *xp,
                hp: Some(combat.hp),
                mp: Some(combat.mp),
                alive: combat.hp != 0,
            },
        })
    }

    #[allow(clippy::too_many_lines)] // One atomic inventory revision and ordered private projection.
    fn apply_grant_v6(
        &mut self,
        tick: Tick,
        command: &ItemGrantCommand,
        previous: Option<Phase4Projection>,
    ) -> Result<Phase4Projection, RejectReason> {
        let id = command.entity;
        let combat = self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or(RejectReason::UnknownEntity)?;
        if !matches!(&combat.role, CombatRole::Player { progression: Some(p), .. } if p.identity.account_id == command.account)
        {
            return Err(RejectReason::NotPermitted);
        }
        let incarnation = if command.paused.is_some() {
            0
        } else {
            combat.incarnation
        };
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        if !items
            .rules
            .parts()
            .capabilities
            .get(&command.template_id)
            .is_some_and(|cap| cap.storable)
        {
            return Err(RejectReason::ItemCapabilityDeferred);
        }
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let revision = if previous.as_ref().is_some_and(|p| !p.ledger.is_empty()) {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let mutation = InventoryMutation::prepare(
            &player.inventory,
            items.canonical.allocator,
            &[InventoryEdit::Add {
                template: command.template_id,
                count: command.count,
            }],
            &items.rules,
            InventoryLimits::from_player(player),
            InventoryBoundary { tick, revision },
        )
        .map_err(|_| RejectReason::InventoryFull)?;
        let items = &mut self
            .phase4
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        items.allocator = mutation.allocator;
        let player = items
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        player.inventory = mutation.inventory;
        player.weight = mutation.weight;
        player.inventory_revision = revision;
        self.rebuild_v5(id).map_err(invalid)?;
        let reference = ItemReceiptRef {
            operation: ItemOperation::Grant,
            key: command.request_id,
        };
        let ordinal = u32::try_from(previous.as_ref().map_or(0, |p| p.receipts.len()))
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or(RejectReason::InvalidLoad)?;
        if self.items_evidence_enabled() && incarnation > 0 {
            let subject = self.binding(id).ok_or(RejectReason::InvalidLoad)?;
            for change in &mutation.changes {
                self.item_evidence_delta(
                    subject,
                    change,
                    ItemOperation::Grant,
                    ordinal,
                    Some(reference),
                    &format!("grant:{}", reference.key),
                );
            }
        }
        let phase = &self
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        let checkpoint = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(id, tick, phase)
            .map_err(invalid)?;
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            enchant_attempts: Vec::new(),
            paused_core: None,
            tick,
            entity: id,
            account: command.account,
            checkpoint: checkpoint.clone(),
            ledger: Vec::new(),
            receipts: Vec::new(),
            materializations: Vec::new(),
            evidence: Vec::new(),
        });
        projection.paused_core = None;
        projection.checkpoint = checkpoint;
        let mut affected = Vec::new();
        for change in mutation.changes {
            let after = change.after.as_ref().ok_or(RejectReason::InvalidLoad)?;
            affected.push(after.id);
            let count_delta = i64::try_from(
                after
                    .count
                    .checked_sub(change.before.as_ref().map_or(0, |i| i.count))
                    .ok_or(RejectReason::InvalidLoad)?,
            )
            .map_err(invalid)?;
            projection.ledger.push(ItemDeltaPlan {
                operation: ItemOperation::Grant,
                operation_ordinal: ordinal,
                instance: after.id,
                before: change.before,
                after: change.after,
                count_delta,
                receipt: Some(reference),
                causes: Vec::new(),
            });
        }
        projection.receipts.push(ItemReceiptFact {
            request_id: command.request_id,
            fingerprint: command.fingerprint.clone(),
            result: FrozenItemResult {
                accepted_sequence: None,
                request_fingerprint: command.fingerprint.clone(),
                entity: id,
                incarnation,
                tick,
                inventory_revision: revision,
                receipt: command.request_id,
                detail: ItemResultDetail::Operation {
                    operation: ItemOperation::Grant,
                    affected,
                    recipe: None,
                },
            },
        });
        projection
            .validate_structure(
                phase
                    .checkpoint(id, tick)
                    .map_err(invalid)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(invalid)?;
        Ok(projection)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm
)]
mod tests {
    use super::*;
    use crate::application::replay_log::AppliedTickRecord;
    use crate::domain::zone::item_grant::grant_fingerprint;
    use crate::domain::{AccountId, CharacterId};

    fn grant(zone: &ZoneState, id: EntityId, key: u128, count: u64) -> ItemGrantCommand {
        ItemGrantCommand {
            entity: id,
            account: AccountId::from_uuid(id.as_uuid()),
            request_id: uuid::Uuid::from_u128(key),
            template_id: 57,
            count,
            fingerprint: grant_fingerprint(CharacterId::from_uuid(id.as_uuid()), 57, count),
            rules_hash: zone.phase4.as_ref().unwrap().rules.hash(),
            paused: None,
            known: None,
            conflict: None,
        }
    }
    fn input(command: &ItemGrantCommand) -> ZoneInput {
        ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(command.clone())))
    }
    fn paused(zone: &ZoneState, id: EntityId) -> ItemPausedCore {
        let entity = &zone.entities[&id];
        let mut load = PlayerLoad::fresh("unused");
        load.checkpoint_revision = Some(4);
        zone.freeze_paused_items_v6(
            zone.next_tick,
            id,
            &ItemPausedCore {
                name: entity.name.clone(),
                position: entity.pos,
                load,
            },
        )
        .unwrap()
    }
    fn projections(tick: &AppliedTick) -> Vec<&Phase4Projection> {
        tick.events
            .iter()
            .filter_map(|event| match event {
                ZoneEvent::ItemCheckpoint(p) => Some(p.as_ref()),
                _ => None,
            })
            .collect()
    }
    fn replay(zone: &mut ZoneState, commands: Vec<ZoneInput>) -> AppliedTick {
        let mut cold = ZoneState::from_snapshot_with_items(
            zone.snapshot(),
            Some(Arc::clone(&zone.phase4.as_ref().unwrap().rules)),
        )
        .unwrap();
        let draft = zone.draft(commands);
        let actual = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(actual, cold.run_tick(draft).unwrap());
        let record = AppliedTickRecord::from_applied(zone.seed.zone, &actual);
        let bytes = record.encode();
        assert_eq!(record, AppliedTickRecord::decode(&bytes).unwrap());
        actual
    }

    #[test]
    fn v6_live_grants_batch_merge_and_scoped_receipts_before_mutation() {
        let (mut zone, owner, _) = ZoneState::item_checkpoint_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let before = zone.phase4.as_ref().unwrap().canonical.clone();
        let first = grant(&zone, owner.entity, 9001, 10);
        let second = grant(&zone, owner.entity, 9002, 21);
        let conflict = grant(&zone, owner.entity, 9001, 11);
        let tick = replay(
            &mut zone,
            vec![
                input(&first),
                input(&first),
                input(&second),
                input(&conflict),
            ],
        );
        assert_eq!(tick.dispositions.len(), 1);
        assert_eq!(tick.dispositions[0].reason, RejectReason::ItemOperationConflict);
        let plans = projections(&tick);
        assert_eq!(plans.len(), 1);
        let plan = plans[0];
        assert!(plan.paused_core.is_none());
        assert!(plan.materializations.is_empty());
        assert_eq!(plan.receipts.len(), 2);
        assert_eq!(plan.ledger.len(), 2);
        assert_eq!(plan.ledger.iter().map(|r| r.count_delta).sum::<i64>(), 31);
        assert_eq!(plan.ledger[0].instance, plan.ledger[1].instance);
        assert_eq!(
            plan.checkpoint.inventory_revision,
            before.players[&owner.entity]
                .inventory_revision
                .checked_add(1)
                .unwrap()
        );
        assert_eq!(
            zone.phase4.as_ref().unwrap().canonical.allocator.bounds().1,
            before.allocator.bounds().1.checked_add(1).unwrap()
        );
        let mut known = first.clone();
        known.known = Some(plan.receipts[0].clone());
        let previous = zone.phase4.as_ref().unwrap().canonical.clone();
        let tick = replay(&mut zone, vec![input(&known)]);
        assert!(tick.dispositions.is_empty());
        assert!(projections(&tick).is_empty());
        assert_eq!(zone.phase4.as_ref().unwrap().canonical, previous);
        let refused = replay(
            &mut zone,
            vec![ZoneInput::session(
                owner.entity,
                SessionGeneration(1),
                1,
                input(&second).command,
            )],
        );
        assert_eq!(refused.dispositions[0].reason, RejectReason::NotPermitted);
    }

    #[test]
    fn v6_offline_grants_preserve_paused_core_without_admission_or_rng() {
        let (mut zone, owner, _) = ZoneState::item_checkpoint_fixture();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let mut core = paused(&zone, owner.entity);
        core.load.alive = false;
        core.load.hp = Some(0);
        replay(
            &mut zone,
            vec![ZoneInput::system(ZoneCommand::Despawn {
                entity: owner.entity,
            })],
        );
        let mut first = grant(&zone, owner.entity, 9011, 10);
        first.paused = Some(Box::new(core.clone()));
        let mut second = grant(&zone, owner.entity, 9012, 21);
        second.paused = first.paused.clone();
        let mut control = zone.clone();
        let empty = control.run_tick(control.draft(Vec::new())).unwrap();
        let tick = replay(&mut zone, vec![input(&first), input(&second)]);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(tick.outputs, empty.outputs);
        let plans = projections(&tick);
        assert_eq!(plans.len(), 1);
        let plan = plans[0];
        let final_core = plan.paused_core.as_ref().unwrap();
        assert!(!final_core.load.alive);
        assert_eq!(final_core.load.hp, Some(0));
        assert_eq!(final_core.position, core.position);
        assert_eq!(final_core.load.mp, core.load.mp);
        assert_eq!(final_core.load.checkpoint_revision, Some(4));
        assert_eq!(plan.receipts.len(), 2);
        assert!(plan.receipts.iter().all(|r| r.result.incarnation == 0));
        assert!(plan.materializations.is_empty());
        assert_eq!(
            plan.checkpoint
                .instances
                .iter()
                .find(|i| i.template_id == 57)
                .unwrap()
                .count,
            31
        );
        assert_eq!(zone.entities, control.entities);
        assert_eq!(
            zone.phase3.as_ref().unwrap().canonical,
            control.phase3.as_ref().unwrap().canonical
        );
        assert_eq!(zone.rng.get_word_pos(), control.rng.get_word_pos());
        let saved = final_core.clone();
        let mut corrupt = first.clone();
        corrupt.request_id = uuid::Uuid::from_u128(9013);
        corrupt
            .paused
            .as_mut()
            .unwrap()
            .load
            .phase4
            .as_mut()
            .unwrap()
            .checkpoint
            .as_mut()
            .unwrap()
            .max_load = -123;
        let allocation = zone.phase4.as_ref().unwrap().canonical.allocator;
        let failed = replay(&mut zone, vec![input(&corrupt)]);
        assert_eq!(failed.dispositions[0].reason, RejectReason::InvalidLoad);
        assert!(projections(&failed).is_empty());
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.allocator, allocation);
        let admitted = replay(
            &mut zone,
            vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: owner.entity,
                name: saved.name.clone(),
                pos: saved.position,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(2),
                load: Some(Box::new(saved.load.clone())),
            })],
        );
        assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
        assert!(zone.entities[&owner.entity].targeting.dead);
        assert!(projections(&admitted).is_empty());
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.allocator, allocation);
    }
}
