//! Strict V6 item intents. Replay tags are independent of WebSocket intent numbers.
use super::{canonical_json, CodecError, PbCommandKind, ZoneCommand};
use crate::domain::zone::{
    FrozenItemResult, ItemCommand, ItemCommandKind, ItemResultDetail, ZoneEvent,
};

pub(super) const fn tag(kind: &ItemCommandKind) -> u32 {
    match kind {
        ItemCommandKind::Equip { .. } => 20,
        ItemCommandKind::Unequip { .. } => 21,
        ItemCommandKind::Use { .. } => 22,
        ItemCommandKind::Destroy { .. } => 23,
        ItemCommandKind::Enchant { .. } => 24,
        ItemCommandKind::ToggleAutoShot { .. } => 25,
        ItemCommandKind::Refine { .. } => 26,
        ItemCommandKind::CancelRefine { .. } => 27,
        ItemCommandKind::ExchangeSpecialAbility { .. } => 28,
        ItemCommandKind::Crystallize { .. } => 29,
    }
}

pub(super) fn command_to_pb(command: &ItemCommand) -> PbCommandKind {
    let bytes = canonical_json(command);
    match command.kind {
        ItemCommandKind::Equip { .. } => PbCommandKind::EquipItem(bytes),
        ItemCommandKind::Unequip { .. } => PbCommandKind::UnequipItem(bytes),
        ItemCommandKind::Use { .. } => PbCommandKind::UseItem(bytes),
        ItemCommandKind::Destroy { .. } => PbCommandKind::DestroyItem(bytes),
        ItemCommandKind::Enchant { .. } => PbCommandKind::EnchantItem(bytes),
        ItemCommandKind::ToggleAutoShot { .. } => PbCommandKind::ToggleAutoShot(bytes),
        ItemCommandKind::Refine { .. } => PbCommandKind::RefineItem(bytes),
        ItemCommandKind::CancelRefine { .. } => PbCommandKind::CancelRefine(bytes),
        ItemCommandKind::ExchangeSpecialAbility { .. } => {
            PbCommandKind::ExchangeSpecialAbility(bytes)
        },
        ItemCommandKind::Crystallize { .. } => PbCommandKind::CrystallizeItem(bytes),
    }
}

pub(super) fn command_from_json(
    bytes: &[u8],
    expected_tag: u32,
) -> Result<ZoneCommand, CodecError> {
    let command: ItemCommand = serde_json::from_slice(bytes).map_err(super::err)?;
    command.authenticated_account().map_err(super::err)?;
    if tag(&command.kind) != expected_tag || canonical_json(&command) != bytes {
        return Err(CodecError("noncanonical or mismatched V6 item intent".into()));
    }
    Ok(ZoneCommand::Item(Box::new(command)))
}

pub(super) fn result_to_pb(result: &FrozenItemResult) -> super::PbOutputItem {
    let bytes = canonical_json(result);
    match result.detail {
        ItemResultDetail::Enchant { .. } => super::PbOutputItem::EnchantResult(bytes),
        ItemResultDetail::Refine { .. } => super::PbOutputItem::RefineResult(bytes),
        ItemResultDetail::Operation { .. } => super::PbOutputItem::ItemOperationResult(bytes),
    }
}

pub(in crate::application::replay_log) fn result_from_json(
    bytes: &[u8],
    expected_tag: u32,
) -> Result<ZoneEvent, CodecError> {
    let result = FrozenItemResult::from_payload(bytes).map_err(super::err)?;
    let tag = match result.detail {
        ItemResultDetail::Enchant { .. } => 37,
        ItemResultDetail::Refine { .. } => 38,
        ItemResultDetail::Operation { .. } => 39,
    };
    if expected_tag != tag || result.incarnation == 0 {
        return Err(CodecError("noncanonical or mismatched live item result".into()));
    }
    Ok(ZoneEvent::ItemResult(Box::new(result)))
}

pub(in crate::application::replay_log) fn grant_from_json(
    bytes: &[u8],
) -> Result<ZoneCommand, CodecError> {
    let command: crate::domain::zone::ItemGrantCommand =
        serde_json::from_slice(bytes).map_err(super::err)?;
    command.validate().map_err(super::err)?;
    if canonical_json(&command) != bytes {
        return Err(CodecError("noncanonical recorded grant".into()));
    }
    Ok(ZoneCommand::MaterializePendingItems(Box::new(command)))
}

pub(super) fn range_from_json(bytes: &[u8]) -> Result<ZoneCommand, CodecError> {
    let range: crate::domain::zone::ItemInstanceRange =
        serde_json::from_slice(bytes).map_err(super::err)?;
    if canonical_json(&range) != bytes {
        return Err(CodecError("noncanonical allocator reservation".into()));
    }
    Ok(ZoneCommand::ExtendItemRange { range })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::zone::{effects::TargetBinding, EntityId, EquipSlot, ItemInstanceId};
    use crate::domain::AccountId;
    use prost::Message as _;

    fn command(kind: ItemCommandKind) -> ItemCommand {
        ItemCommand::new(
            EntityId::from_uuid(uuid::Uuid::from_u128(1)),
            uuid::Uuid::from_u128(2),
            kind,
        )
        .unwrap()
        .with_account(AccountId::from_uuid(uuid::Uuid::from_u128(3)))
        .unwrap()
    }
    #[test]
    fn recorded_grants_use_tag_thirty_with_strict_body_identity() {
        use crate::domain::zone::{AppliedCommand, CommandSource, ItemGrantCommand, Ordinal};
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let grant = ItemGrantCommand {
            entity,
            account: AccountId::from_uuid(uuid::Uuid::from_u128(2)),
            request_id: uuid::Uuid::from_u128(3),
            template_id: 57,
            count: 10,
            fingerprint: crate::domain::zone::item_grant::grant_fingerprint(
                crate::domain::CharacterId::from_uuid(entity.as_uuid()),
                57,
                10,
            ),
            rules_hash: [1; 32],
            paused: None,
            known: None,
            conflict: None,
            enchant: 0,
        };
        let applied = AppliedCommand {
            ordinal: Ordinal(1),
            source: CommandSource::System,
            seq: None,
            command: ZoneCommand::MaterializePendingItems(Box::new(grant.clone())),
        };
        let encoded = super::super::command_to_pb(&applied);
        assert!(matches!(encoded.kind, Some(PbCommandKind::MaterializePendingItems(_))));
        for schema in 3..=8 {
            let record = super::super::PbRecord {
                schema,
                commands: vec![encoded.clone()],
                state_digest: vec![0; 32].into(),
                ..Default::default()
            };
            assert_eq!(super::super::decode_record(&record.encode_to_vec()).is_ok(), schema == 8);
        }
        let bytes = canonical_json(&grant);
        assert_eq!(grant_from_json(&bytes).unwrap(), applied.command);
        let mut whitespace = bytes;
        whitespace.push(b' ');
        assert!(grant_from_json(&whitespace).is_err());
        let mut bad = grant.clone();
        bad.count = 11;
        assert!(grant_from_json(&canonical_json(&bad)).is_err());
        bad = grant.clone();
        bad.request_id = uuid::Uuid::nil();
        assert!(grant_from_json(&canonical_json(&bad)).is_err());
        bad = grant.clone();
        bad.conflict = Some(grant.fingerprint.clone());
        assert!(grant_from_json(&canonical_json(&bad)).is_err());
        let mut value = serde_json::to_value(&grant).unwrap();
        value["unknown"] = serde_json::Value::Null;
        assert!(grant_from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn allocator_reservation_uses_tag_thirty_one_only_in_record_eight() {
        use crate::domain::zone::{AppliedCommand, CommandSource, ItemInstanceRange, Ordinal};
        let range = ItemInstanceRange::new(100, 100, 200).unwrap();
        let command = AppliedCommand {
            ordinal: Ordinal(1),
            source: CommandSource::System,
            seq: None,
            command: ZoneCommand::ExtendItemRange { range },
        };
        let encoded = super::super::command_to_pb(&command);
        assert!(matches!(encoded.kind, Some(PbCommandKind::ExtendItemRange(_))));
        for schema in 3..=8 {
            let record = super::super::PbRecord {
                schema,
                commands: vec![encoded.clone()],
                state_digest: vec![0; 32].into(),
                ..Default::default()
            };
            assert_eq!(super::super::decode_record(&record.encode_to_vec()).is_ok(), schema == 8);
        }
        let bytes = canonical_json(&range);
        assert_eq!(range_from_json(&bytes).unwrap(), command.command);
        let mut whitespace = bytes;
        whitespace.push(b' ');
        assert!(range_from_json(&whitespace).is_err());
        for bad in [
            r#"{"range_start":0,"next_item_instance":0,"range_end_exclusive":200}"#,
            r#"{"range_start":100,"next_item_instance":99,"range_end_exclusive":200}"#,
            r#"{"range_start":100,"next_item_instance":100,"range_end_exclusive":9223372036854775808}"#,
            r#"{"range_start":100,"next_item_instance":100,"range_end_exclusive":200,"unknown":1}"#,
            r#"{"range_start":100,"range_start":101,"next_item_instance":100,"range_end_exclusive":200}"#,
        ] {
            assert!(range_from_json(bad.as_bytes()).is_err());
        }
    }

    #[test]
    fn item_rejections_are_additive_and_record_eight_only() {
        use crate::domain::zone::RejectReason as R;
        for (index, reason) in [
            R::ItemNotOwned,
            R::ItemNotUsable,
            R::InvalidEquipSlot,
            R::InsufficientItems,
            R::InventoryFull,
            R::ItemGradeMismatch,
            R::ItemReuse,
            R::InvalidEnchant,
            R::InvalidRefinement,
            R::InvalidRecipe,
            R::ItemOperationConflict,
            R::ItemCapabilityDeferred,
            R::CrystallizeLevelTooLow,
        ]
        .into_iter()
        .enumerate()
        {
            let tag = 39_i32.checked_add(i32::try_from(index).unwrap()).unwrap();
            assert_eq!(super::super::reason_to_pb(reason), tag);
            assert_eq!(super::super::reason_from_pb(tag).unwrap(), reason);
            assert_eq!(
                i32::from(crate::interface::zone_mapping::reject_reason_to_pb(reason)),
                tag.checked_sub(14).unwrap()
            );
            for schema in 3..=8 {
                let record = super::super::PbRecord {
                    schema,
                    dispositions: vec![super::super::PbDisposition {
                        reason: tag,
                        ..Default::default()
                    }],
                    state_digest: vec![0; 32].into(),
                    ..Default::default()
                };
                assert_eq!(
                    super::super::decode_record(&record.encode_to_vec()).is_ok(),
                    schema == 8
                );
            }
        }
    }

    #[test]
    fn frozen_item_results_pin_tags_and_owner_only_record_eight_routing() {
        use crate::domain::zone::phase4_projection::{ItemEnchantOutcome, ItemRefineOutcome};
        let mut result = crate::domain::zone::phase4_projection::tests::projection()
            .receipts
            .remove(0)
            .result;
        let target = ItemInstanceId::new(7).unwrap();
        let details = [
            (
                37,
                ItemResultDetail::Enchant {
                    target,
                    old_enchant: 3,
                    new_enchant: 4,
                    outcome: ItemEnchantOutcome::Success,
                    refunds: Vec::new(),
                    announcement_tier: 0,
                },
            ),
            (
                38,
                ItemResultDetail::Refine {
                    target,
                    augment: Some(1),
                    outcome: ItemRefineOutcome::Applied,
                },
            ),
            (39, result.detail.clone()),
        ];
        for (tag, detail) in details {
            result.detail = detail;
            let event = ZoneEvent::ItemResult(Box::new(result.clone()));
            let json = canonical_json(&result);
            assert_eq!(result_from_json(&json, tag).unwrap(), event);
            assert!(result_from_json(&json, 40).is_err());
            let bytes = super::super::encode_events(std::slice::from_ref(&event));
            // One output container and exactly the pinned result tag.
            let wire = super::super::PbOutputs::decode(bytes.as_slice()).unwrap();
            assert_eq!(wire.items[0].item, Some(result_to_pb(&result)));
            assert_eq!(super::super::decode_events(&bytes).unwrap(), vec![event.clone()]);
            let mapped = crate::interface::zone_mapping::world_event_to_pb(&event, 0);
            assert_eq!(mapped.len(), 1);
            assert_eq!(
                mapped[0].event,
                Some(crate::interface::zone_mapping::frozen_item_result_to_pb(&result).unwrap())
            );
            for schema in 3..=8 {
                let mut record = super::super::PbRecord {
                    schema,
                    tick: 20,
                    events: bytes.clone().into(),
                    state_digest: vec![0; 32].into(),
                    outputs: vec![super::super::PbPlayerOutput {
                        entity: result.entity.as_uuid().as_bytes().to_vec(),
                        encoded: bytes.clone().into(),
                        sha256: bytes::Bytes::new(),
                    }],
                    ..Default::default()
                };
                // A Known receipt may carry an earlier result tick than this record.
                assert_eq!(
                    super::super::decode_record(&record.encode_to_vec()).is_ok(),
                    schema == 8
                );
                record.outputs[0].entity = uuid::Uuid::from_u128(99).as_bytes().to_vec();
                assert!(super::super::decode_record(&record.encode_to_vec()).is_err());
            }
            let mut whitespace = json.clone();
            whitespace.push(b' ');
            assert!(result_from_json(&whitespace, tag).is_err());
            let unknown = String::from_utf8(json)
                .unwrap()
                .replacen('{', "{\"unexpected\":0,", 1);
            assert!(result_from_json(unknown.as_bytes(), tag).is_err());
            let mut detached = result.clone();
            detached.incarnation = 0;
            assert!(result_from_json(&canonical_json(&detached), tag).is_err());
            detached = result.clone();
            detached.receipt = uuid::Uuid::nil();
            assert!(result_from_json(&canonical_json(&detached), tag).is_err());
        }
    }

    #[test]
    fn item_load_uses_tag_ten_with_strict_cross_sidecar_validation() {
        use crate::domain::zone::{
            AppliedCommand, CommandSource, Ordinal, Phase3Checkpoint, Phase4PlayerLoad, PlayerLoad,
            SessionGeneration, Speed, Vec2Fixed,
        };
        let mut load = PlayerLoad::fresh("human_fighter");
        load.phase3 = Some(Phase3Checkpoint::new());
        load.phase4 = Some(Phase4PlayerLoad {
            checkpoint: Some(
                crate::domain::zone::phase4_projection::tests::projection().checkpoint,
            ),
            pending: Vec::new(),
        });
        let applied = AppliedCommand {
            ordinal: Ordinal(1),
            source: CommandSource::System,
            seq: None,
            command: ZoneCommand::SpawnPlayer {
                entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
                name: "Owner".into(),
                pos: Vec2Fixed::from_tiles(1, 1),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load.clone())),
            },
        };
        let wire = super::super::command_to_pb(&applied);
        assert_eq!(super::super::command_from_pb(wire.clone()).unwrap(), applied);
        for schema in 3..=8 {
            let record = super::super::PbRecord {
                schema,
                commands: vec![wire.clone()],
                state_digest: vec![0; 32].into(),
                ..Default::default()
            };
            assert_eq!(super::super::decode_record(&record.encode_to_vec()).is_ok(), schema == 8);
        }
        let mut spawn = if let Some(PbCommandKind::SpawnPlayer(spawn)) = wire.kind {
            Some(spawn)
        } else {
            None
        }
        .unwrap();
        let encoded_load = spawn.load.as_mut().unwrap();
        assert!(encoded_load.phase4.is_some());
        encoded_load.phase3 = None;
        let mut corrupt = super::super::command_to_pb(&applied);
        corrupt.kind = Some(PbCommandKind::SpawnPlayer(spawn));
        assert!(super::super::command_from_pb(corrupt).is_err());
        load.phase3 = None;
        assert!(load.validate_phase4().is_err());
        let bytes = canonical_json(load.phase4.as_ref().unwrap());
        let mut whitespace = bytes.clone();
        whitespace.push(b' ');
        assert!(super::super::phase4_load_from_json(&whitespace).is_err());
        let unknown = String::from_utf8(bytes)
            .unwrap()
            .replacen('{', "{\"unexpected\":0,", 1);
        assert!(super::super::phase4_load_from_json(unknown.as_bytes()).is_err());
    }

    #[test]
    fn private_item_checkpoint_roundtrips_only_in_v6_event_stream() {
        let projection = crate::domain::zone::phase4_projection::tests::projection();
        let event = crate::domain::zone::ZoneEvent::ItemCheckpoint(Box::new(projection));
        let bytes = super::super::encode_events(std::slice::from_ref(&event));
        assert_eq!(super::super::decode_events(&bytes).unwrap(), vec![event.clone()]);
        assert!(crate::interface::zone_mapping::world_event_to_pb(&event, 0).is_empty());
        for schema in 3..=8 {
            let record = super::super::PbRecord {
                schema,
                tick: 10,
                events: bytes.clone().into(),
                state_digest: vec![0; 32].into(),
                ..Default::default()
            };
            assert_eq!(super::super::decode_record(&record.encode_to_vec()).is_ok(), schema == 8);
        }
        let record = super::super::PbRecord {
            schema: 8,
            tick: 10,
            state_digest: vec![0; 32].into(),
            outputs: vec![super::super::PbPlayerOutput {
                entity: event.entity().as_uuid().as_bytes().to_vec(),
                encoded: bytes.into(),
                sha256: bytes::Bytes::new(),
            }],
            ..Default::default()
        };
        assert!(super::super::decode_record(&record.encode_to_vec()).is_err());
        let reserved = super::super::PbRecord {
            schema: 8,
            state_digest: vec![0; 32].into(),
            events: vec![10, 3, 226, 1, 0].into(),
            ..Default::default()
        };
        assert!(super::super::decode_record(&reserved.encode_to_vec()).is_err());
    }

    #[test]
    fn private_item_checkpoint_rejects_unknown_fields_and_corrupt_invariants() {
        let original = crate::domain::zone::phase4_projection::tests::projection();
        let bytes = canonical_json(&original);
        assert_eq!(super::super::phase4_projection_from_json(&bytes).unwrap(), original);
        let mut whitespace = bytes.clone();
        whitespace.push(b' ');
        assert!(super::super::phase4_projection_from_json(&whitespace).is_err());
        let mut corrupted = original.clone();
        corrupted.checkpoint.ordinary_capacity = u32::MAX;
        assert!(super::super::phase4_projection_from_json(&canonical_json(&corrupted)).is_err());
        corrupted = original.clone();
        corrupted.account = AccountId::from_uuid(uuid::Uuid::nil());
        assert!(super::super::phase4_projection_from_json(&canonical_json(&corrupted)).is_err());
        let mut json = serde_json::to_value(original).unwrap();
        json["checkpoint"]["injected"] = serde_json::Value::Null;
        assert!(super::super::phase4_projection_from_json(&canonical_json(&json)).is_err());
    }

    #[test]
    fn item_intents_have_ten_pinned_tags_and_are_rejected_before_record_eight() {
        let instance = ItemInstanceId::new(1).unwrap();
        let target = ItemInstanceId::new(2).unwrap();
        let npc = TargetBinding {
            entity: EntityId::from_uuid(uuid::Uuid::from_u128(4)),
            incarnation: 1,
        };
        let kinds = [
            ItemCommandKind::Equip {
                instance,
                preferred_slot: Some(EquipSlot::Lfinger),
            },
            ItemCommandKind::Unequip {
                slot: EquipSlot::Rhand,
            },
            ItemCommandKind::Use {
                instance,
                target: Some(npc),
            },
            ItemCommandKind::Destroy { instance, count: 1 },
            ItemCommandKind::Enchant {
                scroll: instance,
                target,
                support: None,
            },
            ItemCommandKind::ToggleAutoShot {
                template: 1835,
                enabled: false,
            },
            ItemCommandKind::Refine {
                npc,
                target,
                stone: instance,
            },
            ItemCommandKind::CancelRefine { npc, target },
            ItemCommandKind::ExchangeSpecialAbility {
                npc,
                target,
                recipe: 1005,
            },
            ItemCommandKind::Crystallize { instance },
        ];
        for (index, kind) in kinds.into_iter().enumerate() {
            let command = command(kind);
            let tag = 20_u32.checked_add(u32::try_from(index).unwrap()).unwrap();
            let bytes = canonical_json(&command);
            assert_eq!(
                command_from_json(&bytes, tag).unwrap(),
                ZoneCommand::Item(Box::new(command.clone()))
            );
            assert!(command_from_json(&bytes, 99).is_err());
            let wire = super::super::PbCommand {
                ordinal: 1,
                session: None,
                seq: None,
                kind: Some(command_to_pb(&command)),
            };
            for schema in 3..=8 {
                let record = super::super::PbRecord {
                    schema,
                    commands: vec![wire.clone()],
                    state_digest: vec![0; 32].into(),
                    ..Default::default()
                };
                assert_eq!(
                    super::super::decode_record(&record.encode_to_vec()).is_ok(),
                    schema == 8
                );
            }
            let duplicate = [wire.encode_to_vec(), wire.encode_to_vec()].concat();
            assert!(super::super::super::codec_validation::item_command(&duplicate).is_err());
        }
    }

    #[test]
    fn item_intent_json_rejects_unknown_duplicate_untrusted_and_noncanonical_payloads() {
        let value = command(ItemCommandKind::Crystallize {
            instance: ItemInstanceId::new(1).unwrap(),
        });
        let mut json = serde_json::to_value(&value).unwrap();
        json["unexpected"] = serde_json::Value::Bool(false);
        assert!(command_from_json(&canonical_json(&json), 29).is_err());
        let mut json = serde_json::to_value(&value).unwrap();
        json.as_object_mut().unwrap().remove("account");
        assert!(command_from_json(&canonical_json(&json), 29).is_err());
        let bytes = canonical_json(&value);
        let mut whitespace = bytes.clone();
        whitespace.push(b' ');
        assert!(command_from_json(&whitespace, 29).is_err());
        let duplicate = String::from_utf8(bytes)
            .unwrap()
            .replacen('{', "{\"account\":null,", 1);
        assert!(command_from_json(duplicate.as_bytes(), 29).is_err());
    }
}
