//! Exact integer item request/result wire shapes. Actor preflight supplies account authority.
use super::{pb, MappingError};
use crate::application::AppError;
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemEnchantOutcome, ItemOperation, ItemRefineOutcome, ItemResultDetail,
};
use crate::domain::zone::{
    effects::TargetBinding, EntityId, EquipSlot, ItemCommand, ItemCommandKind, ItemInstanceId,
};

/// Decode every item intent with the session owner and no trusted account context.
/// Non-item intents return None for the existing command decoder.
#[allow(clippy::too_many_lines)] // Keep the ten exact wire contracts together.
pub fn item_command_from_pb(
    entity: EntityId,
    intent: &pb::client_message::Intent,
) -> Result<Option<ItemCommand>, MappingError> {
    use pb::client_message::Intent;
    let (key, kind) = match intent {
        Intent::EquipItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Equip {
                instance: instance(body.instance_id)?,
                preferred_slot: optional_slot(body.preferred_slot)?,
            },
        ),
        Intent::UnequipItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Unequip {
                slot: optional_slot(body.slot)?.ok_or(MappingError::InvalidItem)?,
            },
        ),
        Intent::UseItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Use {
                instance: instance(body.instance_id)?,
                target: body.target.as_ref().map(binding).transpose()?,
            },
        ),
        Intent::DestroyItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Destroy {
                instance: instance(body.instance_id)?,
                count: body.count,
            },
        ),
        Intent::EnchantItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Enchant {
                scroll: instance(body.scroll_instance_id)?,
                target: instance(body.target_instance_id)?,
                support: body.support_instance_id.map(instance).transpose()?,
            },
        ),
        Intent::ToggleAutoShot(body) => (
            &body.idempotency_key,
            ItemCommandKind::ToggleAutoShot {
                template: body.template_id,
                enabled: body.enabled,
            },
        ),
        Intent::RefineItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Refine {
                npc: required_binding(body.npc.as_ref())?,
                target: instance(body.target_instance_id)?,
                stone: instance(body.stone_instance_id)?,
            },
        ),
        Intent::CancelRefine(body) => (
            &body.idempotency_key,
            ItemCommandKind::CancelRefine {
                npc: required_binding(body.npc.as_ref())?,
                target: instance(body.target_instance_id)?,
            },
        ),
        Intent::ExchangeSpecialAbility(body) => (
            &body.idempotency_key,
            ItemCommandKind::ExchangeSpecialAbility {
                npc: required_binding(body.npc.as_ref())?,
                target: instance(body.target_instance_id)?,
                recipe: body.recipe_id,
            },
        ),
        Intent::CrystallizeItem(body) => (
            &body.idempotency_key,
            ItemCommandKind::Crystallize {
                instance: instance(body.instance_id)?,
            },
        ),
        Intent::MoveTo(_)
        | Intent::StopMove(_)
        | Intent::SetTarget(_)
        | Intent::Attack(_)
        | Intent::StopAttack(_)
        | Intent::Respawn(_)
        | Intent::CastSkill(_)
        | Intent::CancelCast(_)
        | Intent::AcceptResurrect(_) => return Ok(None),
    };
    let key = uuid::Uuid::parse_str(key).map_err(|_| MappingError::InvalidItem)?;
    ItemCommand::new(entity, key, kind)
        .map(Some)
        .map_err(|_| MappingError::InvalidItem)
}
fn instance(value: u64) -> Result<ItemInstanceId, MappingError> {
    ItemInstanceId::new(value).map_err(|_| MappingError::InvalidItem)
}
fn required_binding(value: Option<&pb::ItemTarget>) -> Result<TargetBinding, MappingError> {
    binding(value.ok_or(MappingError::InvalidItem)?)
}
fn binding(value: &pb::ItemTarget) -> Result<TargetBinding, MappingError> {
    let uuid = uuid::Uuid::from_slice(&value.entity_id).map_err(|_| MappingError::InvalidItem)?;
    if uuid.is_nil() || value.incarnation == 0 {
        return Err(MappingError::InvalidItem);
    }
    Ok(TargetBinding {
        entity: EntityId::from_uuid(uuid),
        incarnation: value.incarnation,
    })
}
fn optional_slot(value: i32) -> Result<Option<EquipSlot>, MappingError> {
    Ok(match pb::EquipSlot::try_from(value).map_err(|_| MappingError::InvalidItem)? {
        pb::EquipSlot::Unspecified => None,
        pb::EquipSlot::Under => Some(EquipSlot::Under),
        pb::EquipSlot::Head => Some(EquipSlot::Head),
        pb::EquipSlot::Hair => Some(EquipSlot::Hair),
        pb::EquipSlot::Hair2 => Some(EquipSlot::Hair2),
        pb::EquipSlot::Neck => Some(EquipSlot::Neck),
        pb::EquipSlot::Rhand => Some(EquipSlot::Rhand),
        pb::EquipSlot::Chest => Some(EquipSlot::Chest),
        pb::EquipSlot::Lhand => Some(EquipSlot::Lhand),
        pb::EquipSlot::Rear => Some(EquipSlot::Rear),
        pb::EquipSlot::Lear => Some(EquipSlot::Lear),
        pb::EquipSlot::Gloves => Some(EquipSlot::Gloves),
        pb::EquipSlot::Legs => Some(EquipSlot::Legs),
        pb::EquipSlot::Feet => Some(EquipSlot::Feet),
        pb::EquipSlot::Rfinger => Some(EquipSlot::Rfinger),
        pb::EquipSlot::Lfinger => Some(EquipSlot::Lfinger),
        pb::EquipSlot::Lbracelet => Some(EquipSlot::Lbracelet),
        pb::EquipSlot::Rbracelet => Some(EquipSlot::Rbracelet),
        pb::EquipSlot::Deco1 => Some(EquipSlot::Deco1),
        pb::EquipSlot::Deco2 => Some(EquipSlot::Deco2),
        pb::EquipSlot::Deco3 => Some(EquipSlot::Deco3),
        pb::EquipSlot::Deco4 => Some(EquipSlot::Deco4),
        pb::EquipSlot::Deco5 => Some(EquipSlot::Deco5),
        pb::EquipSlot::Deco6 => Some(EquipSlot::Deco6),
        pb::EquipSlot::Cloak => Some(EquipSlot::Cloak),
        pb::EquipSlot::Belt => Some(EquipSlot::Belt),
    })
}

/// Map validated frozen results to the exact owner-only `WorldEvent` tags 31..33.
pub fn frozen_item_result_to_pb(
    result: &FrozenItemResult,
) -> Result<pb::world_event::Event, AppError> {
    result.validate().map_err(anyhow::Error::from)?;
    let entity_id = result.entity.as_uuid().as_bytes().to_vec();
    let receipt_id = result.receipt.to_string();
    Ok(match &result.detail {
        ItemResultDetail::Operation {
            operation,
            affected,
            recipe,
        } => pb::world_event::Event::ItemOperationResult(pb::ItemOperationResult {
            entity_id,
            incarnation: result.incarnation,
            tick: result.tick.0,
            inventory_revision: result.inventory_revision,
            receipt_id,
            operation: operation_to_pb(*operation)?.into(),
            affected_instance_ids: affected.iter().map(|id| id.get()).collect(),
            recipe_id: *recipe,
        }),
        ItemResultDetail::Enchant {
            target,
            old_enchant,
            new_enchant,
            outcome,
            refunds,
            announcement_tier,
        } => pb::world_event::Event::EnchantResult(pb::EnchantResult {
            entity_id,
            incarnation: result.incarnation,
            tick: result.tick.0,
            inventory_revision: result.inventory_revision,
            receipt_id,
            target_instance_id: target.get(),
            old_enchant: *old_enchant,
            new_enchant: *new_enchant,
            outcome: match outcome {
                ItemEnchantOutcome::Success => pb::EnchantOutcome::Success,
                ItemEnchantOutcome::SafeFailed => pb::EnchantOutcome::SafeFailed,
                ItemEnchantOutcome::BlessedReset => pb::EnchantOutcome::BlessedReset,
                ItemEnchantOutcome::Destroyed => pb::EnchantOutcome::Destroyed,
                ItemEnchantOutcome::SuccessNoIncrement => pb::EnchantOutcome::SuccessNoIncrement,
            }
            .into(),
            refunds: refunds
                .iter()
                .map(|refund| pb::ItemRefund {
                    instance_id: refund.instance.get(),
                    template_id: refund.template,
                    count: refund.count,
                })
                .collect(),
            announcement_tier: *announcement_tier,
        }),
        ItemResultDetail::Refine {
            target,
            augment,
            outcome,
        } => pb::world_event::Event::RefineResult(pb::RefineResult {
            entity_id,
            incarnation: result.incarnation,
            tick: result.tick.0,
            inventory_revision: result.inventory_revision,
            receipt_id,
            target_instance_id: target.get(),
            augment_id: *augment,
            outcome: match outcome {
                ItemRefineOutcome::Applied => pb::RefineOutcome::Applied,
                ItemRefineOutcome::Removed => pb::RefineOutcome::Removed,
            }
            .into(),
        }),
    })
}
fn operation_to_pb(operation: ItemOperation) -> Result<pb::ItemOperation, AppError> {
    Ok(match operation {
        ItemOperation::Grant => pb::ItemOperation::Grant,
        ItemOperation::Starter => pb::ItemOperation::Starter,
        ItemOperation::Backfill => pb::ItemOperation::Backfill,
        ItemOperation::Equip => pb::ItemOperation::Equip,
        ItemOperation::Unequip => pb::ItemOperation::Unequip,
        ItemOperation::Use => pb::ItemOperation::Use,
        ItemOperation::Destroy => pb::ItemOperation::Destroy,
        ItemOperation::Enchant => pb::ItemOperation::Enchant,
        ItemOperation::ToggleAutoShot => pb::ItemOperation::ToggleAutoShot,
        ItemOperation::Refine => pb::ItemOperation::Refine,
        ItemOperation::CancelRefine => pb::ItemOperation::CancelRefine,
        ItemOperation::Exchange => pb::ItemOperation::ExchangeSa,
        ItemOperation::Crystallize => pb::ItemOperation::Crystallize,
        ItemOperation::Consume => {
            return Err(anyhow::anyhow!("automatic consume has no owner operation result").into())
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::too_many_lines)]
mod tests {
    use super::*;
    use prost::Message as _;
    fn target() -> pb::ItemTarget {
        pb::ItemTarget {
            entity_id: uuid::Uuid::from_u128(2).as_bytes().to_vec(),
            incarnation: 3,
        }
    }
    fn owner() -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(1))
    }
    #[test]
    fn all_ten_intents_preserve_every_body_field_without_client_account() {
        use pb::client_message::Intent;
        let key = uuid::Uuid::from_u128(7).to_string();
        let iid = |id| ItemInstanceId::new(id).unwrap();
        let bound = TargetBinding {
            entity: EntityId::from_uuid(uuid::Uuid::from_u128(2)),
            incarnation: 3,
        };
        let cases = [
            (
                Intent::EquipItem(pb::EquipItemRequest {
                    idempotency_key: key.clone(),
                    instance_id: 9,
                    preferred_slot: pb::EquipSlot::Lfinger.into(),
                }),
                ItemCommandKind::Equip {
                    instance: iid(9),
                    preferred_slot: Some(EquipSlot::Lfinger),
                },
            ),
            (
                Intent::UnequipItem(pb::UnequipItemRequest {
                    idempotency_key: key.clone(),
                    slot: pb::EquipSlot::Belt.into(),
                }),
                ItemCommandKind::Unequip {
                    slot: EquipSlot::Belt,
                },
            ),
            (
                Intent::UseItem(pb::UseItemRequest {
                    idempotency_key: key.clone(),
                    instance_id: 9,
                    target: Some(target()),
                }),
                ItemCommandKind::Use {
                    instance: iid(9),
                    target: Some(bound),
                },
            ),
            (
                Intent::DestroyItem(pb::DestroyItemRequest {
                    idempotency_key: key.clone(),
                    instance_id: 9,
                    count: i64::MAX as u64,
                }),
                ItemCommandKind::Destroy {
                    instance: iid(9),
                    count: i64::MAX as u64,
                },
            ),
            (
                Intent::EnchantItem(pb::EnchantItemRequest {
                    idempotency_key: key.clone(),
                    scroll_instance_id: 9,
                    target_instance_id: 10,
                    support_instance_id: Some(11),
                }),
                ItemCommandKind::Enchant {
                    scroll: iid(9),
                    target: iid(10),
                    support: Some(iid(11)),
                },
            ),
            (
                Intent::ToggleAutoShot(pb::ToggleAutoShotRequest {
                    idempotency_key: key.clone(),
                    template_id: 1463,
                    enabled: true,
                }),
                ItemCommandKind::ToggleAutoShot {
                    template: 1463,
                    enabled: true,
                },
            ),
            (
                Intent::RefineItem(pb::RefineItemRequest {
                    idempotency_key: key.clone(),
                    npc: Some(target()),
                    target_instance_id: 9,
                    stone_instance_id: 10,
                }),
                ItemCommandKind::Refine {
                    npc: bound,
                    target: iid(9),
                    stone: iid(10),
                },
            ),
            (
                Intent::CancelRefine(pb::CancelRefineRequest {
                    idempotency_key: key.clone(),
                    npc: Some(target()),
                    target_instance_id: 9,
                }),
                ItemCommandKind::CancelRefine {
                    npc: bound,
                    target: iid(9),
                },
            ),
            (
                Intent::ExchangeSpecialAbility(pb::ExchangeSpecialAbilityRequest {
                    idempotency_key: key.clone(),
                    npc: Some(target()),
                    target_instance_id: 9,
                    recipe_id: 4005,
                }),
                ItemCommandKind::ExchangeSpecialAbility {
                    npc: bound,
                    target: iid(9),
                    recipe: 4005,
                },
            ),
            (
                Intent::CrystallizeItem(pb::CrystallizeItemRequest {
                    idempotency_key: key,
                    instance_id: 9,
                }),
                ItemCommandKind::Crystallize { instance: iid(9) },
            ),
        ];
        for (intent, expected) in cases {
            let wire = pb::ClientMessage {
                seq: 19,
                intent: Some(intent),
            };
            let decoded = pb::ClientMessage::decode(wire.encode_to_vec().as_slice()).unwrap();
            let command = item_command_from_pb(owner(), decoded.intent.as_ref().unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(command.entity, owner());
            assert_eq!(command.request_id, uuid::Uuid::from_u128(7));
            assert_eq!(command.kind, expected);
            assert!(command.authenticated_account().is_err());
        }
        assert_eq!(
            item_command_from_pb(owner(), &Intent::StopMove(pb::StopMoveRequest {})).unwrap(),
            None
        );
        for value in 1..=25 {
            assert_eq!(
                optional_slot(value)
                    .unwrap()
                    .map(|slot| i32::from(slot as u8) + 1),
                Some(value)
            );
        }
        assert_eq!(optional_slot(0).unwrap(), None);
        assert!(optional_slot(26).is_err());
        assert!(optional_slot(-1).is_err());
    }
    #[test]
    fn malformed_keys_slots_counts_ids_and_target_lives_are_invalid() {
        use pb::client_message::Intent;
        let key = uuid::Uuid::from_u128(7).to_string();
        let bad = [
            Intent::EquipItem(pb::EquipItemRequest {
                idempotency_key: "bad".into(),
                instance_id: 1,
                preferred_slot: 0,
            }),
            Intent::EquipItem(pb::EquipItemRequest {
                idempotency_key: uuid::Uuid::nil().to_string(),
                instance_id: 1,
                preferred_slot: 0,
            }),
            Intent::EquipItem(pb::EquipItemRequest {
                idempotency_key: key.clone(),
                instance_id: 0,
                preferred_slot: 0,
            }),
            Intent::EquipItem(pb::EquipItemRequest {
                idempotency_key: key.clone(),
                instance_id: u64::MAX,
                preferred_slot: 0,
            }),
            Intent::EquipItem(pb::EquipItemRequest {
                idempotency_key: key.clone(),
                instance_id: 1,
                preferred_slot: 26,
            }),
            Intent::UnequipItem(pb::UnequipItemRequest {
                idempotency_key: key.clone(),
                slot: 0,
            }),
            Intent::DestroyItem(pb::DestroyItemRequest {
                idempotency_key: key.clone(),
                instance_id: 1,
                count: 0,
            }),
            Intent::DestroyItem(pb::DestroyItemRequest {
                idempotency_key: key.clone(),
                instance_id: 1,
                count: u64::MAX,
            }),
            Intent::EnchantItem(pb::EnchantItemRequest {
                idempotency_key: key.clone(),
                scroll_instance_id: 1,
                target_instance_id: 2,
                support_instance_id: Some(0),
            }),
            Intent::EnchantItem(pb::EnchantItemRequest {
                idempotency_key: key.clone(),
                scroll_instance_id: 1,
                target_instance_id: 1,
                support_instance_id: None,
            }),
            Intent::UseItem(pb::UseItemRequest {
                idempotency_key: key.clone(),
                instance_id: 1,
                target: Some(pb::ItemTarget {
                    entity_id: vec![0; 15],
                    incarnation: 1,
                }),
            }),
            Intent::UseItem(pb::UseItemRequest {
                idempotency_key: key.clone(),
                instance_id: 1,
                target: Some(pb::ItemTarget {
                    incarnation: 0,
                    ..target()
                }),
            }),
            Intent::RefineItem(pb::RefineItemRequest {
                idempotency_key: key.clone(),
                npc: None,
                target_instance_id: 1,
                stone_instance_id: 2,
            }),
            Intent::ToggleAutoShot(pb::ToggleAutoShotRequest {
                idempotency_key: key.clone(),
                template_id: 0,
                enabled: true,
            }),
            Intent::ExchangeSpecialAbility(pb::ExchangeSpecialAbilityRequest {
                idempotency_key: key,
                npc: Some(target()),
                target_instance_id: 1,
                recipe_id: 0,
            }),
        ];
        for intent in bad {
            assert_eq!(item_command_from_pb(owner(), &intent), Err(MappingError::InvalidItem));
        }
    }
    fn result(detail: ItemResultDetail) -> FrozenItemResult {
        FrozenItemResult {
            entity: owner(),
            incarnation: 7,
            tick: crate::domain::zone::Tick(81),
            inventory_revision: 5,
            receipt: uuid::Uuid::from_u128(8),
            detail,
        }
    }
    fn round_trip(result: &FrozenItemResult) -> pb::world_event::Event {
        let event = frozen_item_result_to_pb(result).unwrap();
        let message = pb::WorldEvent {
            event: Some(event.clone()),
        };
        assert_eq!(pb::WorldEvent::decode(message.encode_to_vec().as_slice()).unwrap(), message);
        event
    }
    #[test]
    fn operation_results_include_exact_identity_revision_key_affected_ids_and_recipe() {
        for operation in [
            ItemOperation::Equip,
            ItemOperation::Unequip,
            ItemOperation::Use,
            ItemOperation::Destroy,
            ItemOperation::ToggleAutoShot,
            ItemOperation::Crystallize,
            ItemOperation::Grant,
            ItemOperation::Starter,
            ItemOperation::Backfill,
            ItemOperation::Exchange,
        ] {
            let recipe = (operation == ItemOperation::Exchange).then_some(4005);
            let r = result(ItemResultDetail::Operation {
                operation,
                affected: vec![
                    ItemInstanceId::new(1).unwrap(),
                    ItemInstanceId::new(i64::MAX as u64).unwrap(),
                ],
                recipe,
            });
            let pb::world_event::Event::ItemOperationResult(mapped) = round_trip(&r) else {
                panic!("wrong result shape");
            };
            assert_eq!(
                mapped,
                pb::ItemOperationResult {
                    entity_id: owner().as_uuid().as_bytes().to_vec(),
                    incarnation: 7,
                    tick: 81,
                    inventory_revision: 5,
                    receipt_id: r.receipt.to_string(),
                    operation: operation_to_pb(operation).unwrap().into(),
                    affected_instance_ids: vec![1, i64::MAX as u64],
                    recipe_id: recipe
                }
            );
        }
    }
    #[test]
    fn all_enchant_and_refine_outcomes_preserve_u32_bounds_optional_presence_and_refunds() {
        use crate::domain::zone::phase4_projection::ItemRefund;
        let target = ItemInstanceId::new(1).unwrap();
        for (outcome, old, new) in [
            (ItemEnchantOutcome::Success, 0, 1),
            (ItemEnchantOutcome::SafeFailed, 3, 3),
            (ItemEnchantOutcome::BlessedReset, 9, 0),
            (ItemEnchantOutcome::Destroyed, 9, 0),
            (ItemEnchantOutcome::SuccessNoIncrement, u32::MAX, u32::MAX),
        ] {
            let r = result(ItemResultDetail::Enchant {
                target,
                old_enchant: old,
                new_enchant: new,
                outcome,
                refunds: vec![ItemRefund {
                    instance: ItemInstanceId::new(2).unwrap(),
                    template: 1458,
                    count: i64::MAX as u64,
                }],
                announcement_tier: 2,
            });
            let pb::world_event::Event::EnchantResult(mapped) = round_trip(&r) else {
                panic!("wrong result shape");
            };
            assert_eq!(mapped.entity_id, owner().as_uuid().as_bytes());
            assert_eq!(mapped.incarnation, 7);
            assert_eq!(mapped.tick, 81);
            assert_eq!(mapped.inventory_revision, 5);
            assert_eq!(mapped.receipt_id, r.receipt.to_string());
            assert_eq!(mapped.target_instance_id, 1);
            assert_eq!((mapped.old_enchant, mapped.new_enchant), (old, new));
            assert_ne!(mapped.outcome, 0);
            assert_eq!(
                mapped.refunds,
                vec![pb::ItemRefund {
                    template_id: 1458,
                    count: i64::MAX as u64,
                    instance_id: 2
                }]
            );
            assert_eq!(mapped.announcement_tier, 2);
        }
        for (outcome, augment) in [
            (ItemRefineOutcome::Applied, Some(u32::MAX)),
            (ItemRefineOutcome::Removed, None),
        ] {
            let r = result(ItemResultDetail::Refine {
                target,
                augment,
                outcome,
            });
            let pb::world_event::Event::RefineResult(mapped) = round_trip(&r) else {
                panic!("wrong result shape");
            };
            assert_eq!(mapped.entity_id, owner().as_uuid().as_bytes());
            assert_eq!(mapped.incarnation, 7);
            assert_eq!(mapped.tick, 81);
            assert_eq!(mapped.inventory_revision, 5);
            assert_eq!(mapped.receipt_id, r.receipt.to_string());
            assert_eq!(mapped.target_instance_id, 1);
            assert_eq!(mapped.augment_id, augment);
            assert_ne!(mapped.outcome, 0);
        }
    }
}
