//! Strict replay tags32–36; only the equipment payload can cross owner boundaries.
use super::{canonical_json, CodecError, PbOutputItem};
use crate::domain::zone::item_views::{ItemViewDetail, ItemViewEvent};
use crate::domain::zone::ZoneEvent;

pub(super) const fn tag(detail: &ItemViewDetail) -> u32 {
    match detail {
        ItemViewDetail::InventorySync { .. } => 32,
        ItemViewDetail::InventoryChanged { .. } => 33,
        ItemViewDetail::EquipmentChanged { .. } => 34,
        ItemViewDetail::WeightChanged { .. } => 35,
        ItemViewDetail::PenaltyChanged { .. } => 36,
    }
}
pub(super) fn to_pb(event: &ItemViewEvent) -> PbOutputItem {
    let bytes = canonical_json(event);
    match event.detail {
        ItemViewDetail::InventorySync { .. } => PbOutputItem::InventorySync(bytes),
        ItemViewDetail::InventoryChanged { .. } => PbOutputItem::InventoryChanged(bytes),
        ItemViewDetail::EquipmentChanged { .. } => PbOutputItem::EquipmentChanged(bytes),
        ItemViewDetail::WeightChanged { .. } => PbOutputItem::WeightChanged(bytes),
        ItemViewDetail::PenaltyChanged { .. } => PbOutputItem::PenaltyChanged(bytes),
    }
}
pub(in crate::application::replay_log) fn from_json(
    bytes: &[u8],
    expected: u32,
) -> Result<ZoneEvent, CodecError> {
    let event: ItemViewEvent = serde_json::from_slice(bytes).map_err(super::err)?;
    event.validate().map_err(super::err)?;
    if tag(&event.detail) != expected || canonical_json(&event) != bytes {
        return Err(CodecError("noncanonical or mismatched item view".into()));
    }
    Ok(ZoneEvent::ItemView(Box::new(event)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]
mod tests {
    use super::*;
    use crate::domain::zone::item_views::{tests::inventory, VisibleEquipment};
    use crate::domain::zone::{EntityId, EquipSlot, Tick};
    use prost::Message as _;
    #[test]
    fn inventory_view_tags_privacy_and_wire_mapping_are_record_eight_only() {
        let before = inventory();
        let mut after = before.clone();
        after.inventory_revision = 2;
        after.items[0].revision = 2;
        let details = [
            ItemViewDetail::InventorySync {
                baseline_tick: Tick(5),
                inventory: before.clone(),
            },
            crate::domain::zone::item_views::inventory_delta(&before, &after)
                .unwrap()
                .unwrap(),
            ItemViewDetail::EquipmentChanged {
                visible: vec![VisibleEquipment {
                    slot: EquipSlot::Rhand,
                    template_id: 0,
                    glow_tier: 0,
                }],
            },
            ItemViewDetail::WeightChanged {
                inventory_revision: 1,
                weight: before.weight,
            },
            ItemViewDetail::PenaltyChanged {
                inventory_revision: 1,
                penalties: before.penalties,
            },
        ];
        for (index, detail) in details.into_iter().enumerate() {
            let expected = 32 + u32::try_from(index).unwrap();
            let view = ItemViewEvent {
                entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
                incarnation: 1,
                tick: Tick(5),
                detail,
            };
            assert_eq!(tag(&view.detail), expected);
            let event = ZoneEvent::ItemView(Box::new(view.clone()));
            let json = canonical_json(&view);
            assert_eq!(from_json(&json, expected).unwrap(), event);
            assert!(from_json(&json, 37).is_err());
            let bytes = super::super::encode_events(std::slice::from_ref(&event));
            assert_eq!(super::super::decode_events(&bytes).unwrap(), vec![event.clone()]);
            let mapped = crate::interface::zone_mapping::world_event_to_pb(&event, 0);
            assert_eq!(mapped.len(), 1);
            let public = !view.owner_only();
            if public {
                let crate::interface::grpc::pb::world_event::Event::EquipmentChanged(gear) =
                    mapped[0].event.as_ref().unwrap()
                else {
                    unreachable!();
                };
                assert_eq!(gear.visible[0].template_id, 0);
                assert_eq!(gear.visible[0].slot, 6);
            }
            for schema in 3..=8 {
                let mut record = super::super::PbRecord {
                    schema,
                    tick: 5,
                    events: bytes.clone().into(),
                    state_digest: vec![0; 32].into(),
                    outputs: vec![super::super::PbPlayerOutput {
                        entity: view.entity.as_uuid().as_bytes().to_vec(),
                        encoded: bytes.clone().into(),
                        sha256: bytes::Bytes::new(),
                    }],
                    ..Default::default()
                };
                assert_eq!(
                    super::super::decode_record(&record.encode_to_vec()).is_ok(),
                    schema == 8
                );
                record.outputs[0].entity = uuid::Uuid::from_u128(99).as_bytes().to_vec();
                assert_eq!(
                    super::super::decode_record(&record.encode_to_vec()).is_ok(),
                    schema == 8 && public
                );
            }
            let mut whitespace = json.clone();
            whitespace.push(b' ');
            assert!(from_json(&whitespace, expected).is_err());
            let unknown = String::from_utf8(json)
                .unwrap()
                .replacen('{', "{\"unexpected\":0,", 1);
            assert!(from_json(unknown.as_bytes(), expected).is_err());
            let mut detached = view;
            detached.incarnation = 0;
            assert!(from_json(&canonical_json(&detached), expected).is_err());
        }
    }
}
