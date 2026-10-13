//! Recorded first-kit admission. The enclosing admission candidate owns every allocation.
use super::*;
use crate::domain::zone::item_inventory::{
    InventoryBoundary, InventoryEdit, InventoryLimits, InventoryMutation,
};
use crate::domain::zone::phase4_admission::{
    ItemKitManifest, ItemMaterialization, PendingItemMaterialization,
};
use crate::domain::zone::phase4_checkpoint::StarterKitReceipt;
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemDeltaPlan, ItemMaterializationFact, ItemOperation, ItemReceiptFact,
    ItemReceiptRef, ItemResultDetail,
};
use crate::domain::zone::{ItemError, Phase4Player, Phase4Projection};

fn kit(
    load: &PlayerLoad,
) -> Result<Option<(&PendingItemMaterialization, &ItemKitManifest, ItemOperation)>, ItemError> {
    let Some(items) = &load.phase4 else {
        return Ok(None);
    };
    if items.pending.is_empty() {
        return Ok(None);
    }
    if items.checkpoint.is_some() || items.pending.len() != 1 {
        return Err(ItemError("first kit requires one request and no retained inventory"));
    }
    let request = items
        .pending
        .first()
        .ok_or(ItemError("missing kit request"))?;
    let (manifest, operation) = match &request.materialization {
        ItemMaterialization::Starter(manifest) => (manifest, ItemOperation::Starter),
        ItemMaterialization::Backfill(manifest) => (manifest, ItemOperation::Backfill),
        ItemMaterialization::Grant(_) => {
            return Err(ItemError("direct grants are not pending kits"))
        },
    };
    Ok(Some((request, manifest, operation)))
}

impl ZoneState {
    pub(super) fn validate_pending_kit_v6(&self, load: &PlayerLoad) -> Result<(), ItemError> {
        let Some((_, manifest, _)) = kit(load)? else {
            return Ok(());
        };
        let identity = &load
            .progression
            .as_ref()
            .ok_or(ItemError("kit owner missing"))?
            .identity;
        if manifest.source_class_id != identity.base_class_id {
            return Err(ItemError("kit lineage differs from character creation class"));
        }
        let rules = &self
            .phase4
            .as_ref()
            .ok_or(ItemError("item epoch missing"))?
            .rules;
        let expected = rules
            .parts()
            .starter_kits
            .iter()
            .find(|row| row.class_id == identity.base_class_id.0)
            .ok_or(ItemError("kit lineage absent from rules"))?;
        if manifest.items.len() != expected.items.len()
            || manifest
                .items
                .iter()
                .zip(&expected.items)
                .any(|(actual, expected)| {
                    (actual.template_id, actual.count, actual.equipped)
                        != (expected.template_id, expected.count, expected.equipped)
                })
        {
            return Err(ItemError("kit contents differ from hash-bound source row order"));
        }
        Ok(())
    }

    /// Install a temporary empty owner before the first fold. It remains private to admission.
    pub(super) fn begin_pending_kit_v6(
        &mut self,
        id: EntityId,
        load: Option<&PlayerLoad>,
    ) -> Result<bool, TickError> {
        let Some(load) = load else { return Ok(false) };
        let Some((request, manifest, _)) = kit(load).map_err(|_| TickError::Phase3Invariant)?
        else {
            return Ok(false);
        };
        self.validate_pending_kit_v6(load)
            .map_err(|_| TickError::Phase3Invariant)?;
        let items = &mut self
            .phase4
            .as_mut()
            .ok_or(TickError::Phase3Invariant)?
            .canonical;
        if items.players.contains_key(&id) {
            return Err(TickError::Phase3Invariant);
        }
        items.players.insert(
            id,
            Phase4Player {
                inventory: BTreeMap::new(),
                paperdoll: BTreeMap::new(),
                auto_shots: std::collections::BTreeSet::new(),
                item_reuse: Vec::new(),
                equipment_sources: BTreeMap::new(),
                active_sets: BTreeMap::new(),
                weight: 0,
                max_load: 0,
                ordinary_capacity: 0,
                quest_capacity: 0,
                weight_penalty_bonus: 0,
                overload_level: 0,
                overloaded: false,
                expertise: 0,
                weapon_penalty: 0,
                armor_penalty: 0,
                inventory_revision: 1,
                starter_kit_receipt: StarterKitReceipt {
                    manifest_version: request.manifest_version,
                    manifest_hash: request.manifest_hash,
                    source_class: manifest.source_class_id,
                    request_id: request.request_id,
                    materialized_revision: 1,
                },
            },
        );
        Ok(true)
    }

    /// Preserve XML grant/equip order; deferred tutorial items remain stored in the kit.
    #[allow(clippy::too_many_lines)]
    pub(super) fn materialize_kit_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        load: Option<&PlayerLoad>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let Some(load) = load else { return Ok(()) };
        let Some((request, manifest, operation)) =
            kit(load).map_err(|_| TickError::Phase3Invariant)?
        else {
            return Ok(());
        };
        let rules = Arc::clone(
            &self
                .phase4
                .as_ref()
                .ok_or(TickError::Phase3Invariant)?
                .rules,
        );
        for row in &manifest.items {
            let items = &self
                .phase4
                .as_ref()
                .ok_or(TickError::Phase3Invariant)?
                .canonical;
            let player = items.players.get(&id).ok_or(TickError::Phase3Invariant)?;
            let mutation = InventoryMutation::prepare(
                &player.inventory,
                items.allocator,
                &[InventoryEdit::Add {
                    template: row.template_id,
                    count: row.count,
                }],
                &rules,
                InventoryLimits::from_player(player),
                InventoryBoundary {
                    tick,
                    revision: 1,
                    origin: crate::domain::zone::ItemCreationOrigin {
                        zone: self.seed.zone,
                        epoch: self.seed.epoch,
                    },
                },
            )
            .map_err(|_| TickError::Phase3Invariant)?;
            let granted: Vec<_> = mutation
                .changes
                .iter()
                .filter_map(|change| change.after.as_ref().map(|item| item.id))
                .collect();
            let items = &mut self
                .phase4
                .as_mut()
                .ok_or(TickError::Phase3Invariant)?
                .canonical;
            items.allocator = mutation.allocator;
            let player = items
                .players
                .get_mut(&id)
                .ok_or(TickError::Phase3Invariant)?;
            player.inventory = mutation.inventory;
            player.weight = mutation.weight;
            if row.equipped {
                for instance in granted {
                    let player = self
                        .phase4
                        .as_ref()
                        .ok_or(TickError::Phase3Invariant)?
                        .canonical
                        .players
                        .get(&id)
                        .ok_or(TickError::Phase3Invariant)?;
                    let mutation = crate::domain::zone::item_equipment::PaperdollMutation::equip(
                        &player.inventory,
                        &player.paperdoll,
                        instance,
                        None,
                        &rules,
                        1,
                    )
                    .map_err(|_| TickError::Phase3Invariant)?;
                    let next = self
                        .phase3
                        .as_ref()
                        .ok_or(TickError::Phase3Invariant)?
                        .canonical
                        .next_install_sequence;
                    let (player, next) = crate::domain::zone::item_sources::apply_paperdoll(
                        player, &mutation, &rules, next,
                    )
                    .map_err(|_| TickError::Phase3Invariant)?;
                    self.phase4
                        .as_mut()
                        .ok_or(TickError::Phase3Invariant)?
                        .canonical
                        .players
                        .insert(id, player);
                    self.phase3
                        .as_mut()
                        .ok_or(TickError::Phase3Invariant)?
                        .canonical
                        .next_install_sequence = next;
                    self.rebuild_v5(id)?;
                }
            }
            self.rebuild_v5(id)?;
        }
        // Only unspecified fresh resources fill to the final equipped maximum. Backfill
        // preserves its explicitly loaded HP/MP and the ordinary clamp-without-healing rule.
        let combat = self
            .entities
            .get_mut(&id)
            .and_then(|entity| entity.combat.as_mut())
            .ok_or(TickError::Phase3Invariant)?;
        if load.alive && load.hp.is_none() {
            combat.hp = combat.sheet.max_hp();
        }
        if load.mp.is_none() {
            combat.mp = combat.sheet.max_mp();
        }
        let incarnation = combat.incarnation;
        self.rebuild_v5(id)?;
        let phase = &self
            .phase3
            .as_ref()
            .ok_or(TickError::Phase3Invariant)?
            .canonical;
        let checkpoint = self
            .phase4
            .as_ref()
            .ok_or(TickError::Phase3Invariant)?
            .canonical
            .checkpoint(id, tick, phase)
            .map_err(|_| TickError::Phase3Invariant)?;
        let reference = ItemReceiptRef {
            operation,
            key: request.request_id,
        };
        let result = FrozenItemResult {
            accepted_sequence: None,
            request_fingerprint: request.request_fingerprint.clone(),
            entity: id,
            incarnation,
            tick,
            inventory_revision: 1,
            receipt: request.request_id,
            detail: ItemResultDetail::Operation {
                operation,
                affected: checkpoint.instances.iter().map(|item| item.id).collect(),
                recipe: None,
            },
        };
        let ledger = checkpoint
            .instances
            .iter()
            .map(|item| {
                Ok(ItemDeltaPlan {
                    receipt_refs: Vec::new(),
                    operation,
                    operation_ordinal: 1,
                    instance: item.id,
                    before: None,
                    after: Some(item.clone()),
                    count_delta: i64::try_from(item.count)
                        .map_err(|_| TickError::Phase3Invariant)?,
                    receipt: Some(reference),
                    causes: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, TickError>>()?;
        let projection = Phase4Projection {
            enchant_attempts: Vec::new(),
            exchanges: Vec::new(),
            refines: Vec::new(),
            paused_core: None,
            tick,
            entity: id,
            account: load
                .progression
                .as_ref()
                .ok_or(TickError::Phase3Invariant)?
                .identity
                .account_id,
            checkpoint,
            ledger,
            receipts: vec![ItemReceiptFact {
                request_id: request.request_id,
                fingerprint: request.request_fingerprint.clone(),
                result,
            }],
            materializations: vec![ItemMaterializationFact {
                request_id: request.request_id,
                manifest_version: request.manifest_version,
                manifest_hash: request.manifest_hash,
                inventory_revision: 1,
                receipt: reference,
            }],
            evidence: Vec::new(),
        };
        projection
            .validate_structure(
                phase
                    .checkpoint(id, tick)
                    .map_err(|_| TickError::Phase3Invariant)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(|_| TickError::Phase3Invariant)?;
        if self.items_evidence_enabled() {
            let subject = self.binding(id).ok_or(TickError::Phase3Invariant)?;
            for row in &projection.ledger {
                self.item_evidence_delta(
                    subject,
                    &crate::domain::zone::item_inventory::InventoryChange {
                        before: row.before.clone(),
                        after: row.after.clone(),
                    },
                    row.operation,
                    row.operation_ordinal,
                    row.receipt,
                    &format!("materialize:{}", reference.key),
                );
            }
            self.item_evidence_baseline(id)?;
        }
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, ClassState,
    };
    use crate::domain::zone::phase4_admission::StarterKitSeed;
    use crate::domain::zone::{
        ItemInstanceRange, Phase3Checkpoint, Phase4PlayerLoad, PlayerProgression,
    };
    use crate::domain::{class::ClassId, AccountId};
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        rules_data::{load_rules, RulesSource},
    };

    #[test]
    fn v6_pending_kits_materialize_every_playable_lineage_once_before_baseline() {
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let base = ZoneState::new_phase4(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            load_rules(&RulesSource::embedded()).unwrap().rules,
            Arc::clone(&classes),
            (*bundle.skills).clone(),
            Arc::clone(&bundle.items),
            ItemInstanceRange::new(1, 1, 10000).unwrap(),
        )
        .unwrap();
        let id = EntityId::from_uuid(uuid::Uuid::from_u128(71));
        let mut lineages = 0;
        for source in &bundle.items.parts().starter_kits {
            let Some(class) = classes.get(ClassId(source.class_id)) else {
                continue;
            };
            lineages += 1;
            for backfill in [false, true] {
                let mut zone = base.clone();
                let manifest = ItemKitManifest {
                    source_class_id: class.id,
                    items: source
                        .items
                        .iter()
                        .map(|row| StarterKitSeed {
                            template_id: row.template_id,
                            count: row.count,
                            equipped: row.equipped,
                        })
                        .collect(),
                };
                let materialization = if backfill {
                    ItemMaterialization::Backfill(manifest)
                } else {
                    ItemMaterialization::Starter(manifest)
                };
                let request = PendingItemMaterialization {
                    request_id: uuid::Uuid::from_u128(900),
                    request_fingerprint: "source-kit-fixture".into(),
                    manifest_version: 1,
                    manifest_hash: materialization.manifest_hash().unwrap(),
                    materialization,
                };
                let mut load = PlayerLoad::fresh(
                    crate::domain::character_progression::base_class_profile(class.id).unwrap(),
                );
                load.phase3 = Some(Phase3Checkpoint::new());
                load.phase4 = Some(Phase4PlayerLoad {
                    checkpoint: None,
                    pending: vec![request],
                });
                load.checkpoint_revision = Some(0);
                load.progression = Some(Box::new(PlayerProgression {
                    identity: CharacterIdentity {
                        account_id: AccountId::from_uuid(uuid::Uuid::from_u128(72)),
                        race: class.race,
                        base_class_id: class.id,
                        appearance: CharacterAppearance::default(),
                    },
                    class_state: ClassState::new(class.id),
                    max_cp: 0,
                }));
                if backfill {
                    load.hp = Some(0);
                    load.mp = Some(1);
                    load.alive = false;
                }
                let spawn = |load: PlayerLoad, generation| {
                    ZoneInput::system(ZoneCommand::SpawnPlayer {
                        entity: id,
                        name: "Kit".into(),
                        pos: Vec2Fixed::from_tiles(10, 10),
                        speed: Speed::DEFAULT,
                        generation: SessionGeneration(generation),
                        load: Some(Box::new(load)),
                    })
                };
                let rng = zone.rng.get_word_pos();
                let mut invalid = load.clone();
                let request = &mut invalid.phase4.as_mut().unwrap().pending[0];
                if let ItemMaterialization::Starter(manifest)
                | ItemMaterialization::Backfill(manifest) = &mut request.materialization
                {
                    manifest.items[0].count += 1;
                }
                request.manifest_hash = request.materialization.manifest_hash().unwrap();
                let allocation = zone.phase4.as_ref().unwrap().canonical.allocator;
                let sequence = zone
                    .phase3
                    .as_ref()
                    .unwrap()
                    .canonical
                    .next_install_sequence;
                let rejected = zone.run_tick(zone.draft(vec![spawn(invalid, 1)])).unwrap();
                assert_eq!(rejected.dispositions.len(), 1);
                assert_eq!(rejected.dispositions[0].reason, RejectReason::InvalidLoad);
                assert!(zone.entities.is_empty());
                assert!(rejected.events.is_empty());
                assert_eq!(zone.phase4.as_ref().unwrap().canonical.allocator, allocation);
                assert_eq!(
                    zone.phase3
                        .as_ref()
                        .unwrap()
                        .canonical
                        .next_install_sequence,
                    sequence
                );
                assert_eq!(zone.rng.get_word_pos(), rng);
                if class.id == ClassId(0) && !backfill {
                    let mut exhausted = zone.clone();
                    exhausted.phase4.as_mut().unwrap().canonical.allocator =
                        ItemInstanceRange::new(1, 1, 2).unwrap();
                    let before = exhausted.phase4.as_ref().unwrap().canonical.clone();
                    let rejected = exhausted
                        .run_tick(exhausted.draft(vec![spawn(load.clone(), 1)]))
                        .unwrap();
                    assert_eq!(rejected.dispositions[0].reason, RejectReason::InvalidLoad);
                    assert!(rejected.events.is_empty());
                    assert!(exhausted.entities.is_empty());
                    assert_eq!(exhausted.phase4.as_ref().unwrap().canonical, before);
                    assert_eq!(
                        exhausted
                            .phase3
                            .as_ref()
                            .unwrap()
                            .canonical
                            .next_install_sequence,
                        sequence
                    );
                    assert_eq!(exhausted.rng.get_word_pos(), rng);
                }
                let mut cold = ZoneState::from_snapshot_with_items(
                    zone.snapshot(),
                    Some(Arc::clone(&bundle.items)),
                )
                .unwrap();
                let draft = zone.draft(vec![spawn(load.clone(), 1)]);
                let applied = zone.run_tick(draft.clone()).unwrap();
                assert!(
                    applied.dispositions.is_empty(),
                    "class {}: {:?}",
                    class.id.0,
                    applied.dispositions
                );
                assert_eq!(applied, cold.run_tick(draft).unwrap());
                assert_eq!(zone.rng.get_word_pos(), rng);
                let items = &zone.phase4.as_ref().unwrap().canonical;
                let player = &items.players[&id];
                let mut counts = BTreeMap::new();
                for item in player.inventory.values() {
                    *counts.entry(item.template_id).or_insert(0_u64) += item.count;
                }
                let mut expected = BTreeMap::new();
                for row in &source.items {
                    *expected.entry(row.template_id).or_insert(0_u64) += row.count;
                }
                assert_eq!(counts, expected);
                for row in source.items.iter().filter(|row| row.equipped) {
                    assert!(player
                        .inventory
                        .values()
                        .any(|item| item.template_id == row.template_id && item.slot.is_some()));
                }
                assert_eq!(player.inventory_revision, 1);
                assert_eq!(player.starter_kit_receipt.source_class, class.id);
                assert_eq!(
                    player.ordinary_capacity,
                    if class.race == crate::domain::Race::Dwarf {
                        100
                    } else {
                        80
                    }
                );
                assert_eq!(zone.entities[&id].combat.as_ref().unwrap().hp == 0, backfill);
                let projections: Vec<_> = applied
                    .events
                    .iter()
                    .filter_map(|event| {
                        if let ZoneEvent::ItemCheckpoint(projection) = event {
                            Some(projection)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(projections.len(), 1);
                let projection = projections[0];
                assert_eq!(projection.ledger.len(), player.inventory.len());
                assert!(projection.ledger.iter().all(|row| row.before.is_none()
                    && row.operation
                        == if backfill {
                            ItemOperation::Backfill
                        } else {
                            ItemOperation::Starter
                        }));
                assert_eq!(projection.materializations.len(), 1);
                assert_eq!(projection.receipts.len(), 1);
                assert!(applied.outputs[&id].iter().any(|output| matches!(output, ObserverOutput::Event(ZoneEvent::ItemView(view)) if matches!(view.detail, crate::domain::zone::item_views::ItemViewDetail::InventorySync { .. }))));
                let record = crate::application::replay_log::AppliedTickRecord::from_applied(
                    zone.seed.zone,
                    &applied,
                );
                assert_eq!(
                    crate::application::replay_log::AppliedTickRecord::decode(&record.encode())
                        .unwrap(),
                    record
                );
                let allocation = items.allocator;
                let checkpoint = projection.checkpoint.clone();
                let phase3 = zone
                    .phase3
                    .as_ref()
                    .unwrap()
                    .canonical
                    .checkpoint(id, applied.tick)
                    .unwrap();
                zone.run_tick(
                    zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: id })]),
                )
                .unwrap();
                load.phase3 = Some(phase3);
                load.phase4 = Some(Phase4PlayerLoad {
                    checkpoint: Some(checkpoint.clone()),
                    pending: Vec::new(),
                });
                let reapplied = zone.run_tick(zone.draft(vec![spawn(load, 2)])).unwrap();
                assert!(reapplied.dispositions.is_empty(), "{:?}", reapplied.dispositions);
                assert!(!reapplied
                    .events
                    .iter()
                    .any(|event| matches!(event, ZoneEvent::ItemCheckpoint(_))));
                assert_eq!(zone.phase4.as_ref().unwrap().canonical.allocator, allocation);
                assert_eq!(
                    zone.phase4.as_ref().unwrap().canonical.players[&id]
                        .inventory
                        .values()
                        .cloned()
                        .collect::<Vec<_>>(),
                    checkpoint.instances
                );
                let restored = ZoneState::from_snapshot_with_items(
                    zone.snapshot(),
                    Some(Arc::clone(&bundle.items)),
                )
                .unwrap();
                assert_eq!(restored.state_digest(), zone.state_digest());
            }
        }
        assert_eq!(lineages, 9);
    }
}
