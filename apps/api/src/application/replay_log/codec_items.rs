//! Strict V6 item intents. Replay tags are independent of WebSocket intent numbers.
use super::{canonical_json, CodecError, PbCommandKind, ZoneCommand};
use crate::domain::zone::{ItemCommand, ItemCommandKind};

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
