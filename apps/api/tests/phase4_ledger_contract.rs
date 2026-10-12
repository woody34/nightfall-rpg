//! API ledger identity, explicit consumption deltas and signed storage boundary contracts.
#![allow(missing_docs, clippy::unwrap_used)]
use nightfall_api::application::ports::{ItemLedgerDelta, ItemLedgerEntry, ItemLedgerOperation};
use nightfall_api::domain::zone::phase4_projection::{ItemAuditEvidence, ItemEvidenceValue};
use nightfall_api::domain::{
    zone::{ItemInstance, ItemInstanceId, ItemLocation, Tick, ZoneId},
    CharacterId,
};
use std::collections::BTreeMap;

fn cause(family: &str, ordinal: u64, debit: i64, receipt: Option<uuid::Uuid>) -> serde_json::Value {
    let mut fields = BTreeMap::from([
        ("operation_ordinal".into(), ItemEvidenceValue::Unsigned(ordinal)),
        ("count_delta".into(), ItemEvidenceValue::Signed(debit)),
    ]);
    if let Some(receipt) = receipt {
        fields.insert("receipt".into(), ItemEvidenceValue::Text(receipt.to_string()));
    } else {
        fields
            .insert("cast_id".into(), ItemEvidenceValue::Unsigned(ordinal.checked_add(1).unwrap()));
    }
    serde_json::to_value(ItemAuditEvidence {
        family: family.into(),
        fields,
    })
    .unwrap()
}

fn entry() -> ItemLedgerEntry {
    let before = ItemInstance {
        creation_origin: None,
        id: ItemInstanceId::new(1).unwrap(),
        template_id: 1835,
        count: 10,
        location: ItemLocation::Inventory,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(0),
        revision: 1,
    };
    let mut after = before.clone();
    after.count = 4;
    after.revision = 2;
    ItemLedgerEntry {
        character_id: CharacterId::new(),
        zone_id: ZoneId(1),
        epoch: 1,
        tick: Tick(1),
        revision: 2,
        operation: ItemLedgerOperation::Consume,
        instance_id: before.id,
        operation_ordinal: 0,
        delta: ItemLedgerDelta {
            receipt_refs: Vec::new(),
            receipt: None,
            before: Some(before),
            after: Some(after),
            count_delta: -6,
            causes: vec![
                cause("skill_cast_payment", 0, -2, None),
                cause("skill_cast_payment", 2, -4, None),
            ],
        },
        receipt_ref: None,
    }
}
#[test]
fn aggregate_scoped_receipts_preserve_order_and_cannot_claim_one_false_sql_reference() {
    use nightfall_api::domain::zone::phase4_projection::{ItemOperation, ItemReceiptRef};
    let first = ItemReceiptRef {
        operation: ItemOperation::Exchange,
        key: uuid::Uuid::from_u128(7),
    };
    let second = ItemReceiptRef {
        operation: ItemOperation::Enchant,
        key: uuid::Uuid::from_u128(8),
    };
    let mut row = entry();
    row.delta.receipt_refs = vec![first, second];
    row.delta.causes = vec![
        cause("exchange_payment", 0, -2, Some(first.key)),
        cause("enchant_payment", 2, -4, Some(second.key)),
    ];
    row.validate().unwrap();
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(
        json.get("delta")
            .unwrap()
            .get("receipt_refs")
            .unwrap()
            .clone(),
        serde_json::to_value([first, second]).unwrap()
    );
    assert!(json.get("receipt_ref").unwrap().is_null());
    for mode in 0..3 {
        let mut bad = row.clone();
        match mode {
            0 => bad.delta.receipt_refs.push(first),
            1 => bad.operation = ItemLedgerOperation::Grant,
            _ => {
                bad.delta.receipt = Some(first);
                bad.receipt_ref = Some(first.key);
            },
        }
        assert!(bad.validate().is_err());
    }
    row.delta.receipt_refs.clear();
    assert!(serde_json::to_value(row)
        .unwrap()
        .get("delta")
        .unwrap()
        .get("receipt_refs")
        .is_none());
}
#[test]
fn canonical_consume_endpoint_matches_debit_and_typed_roundtrip() {
    // D66 emits the Consume aggregate last; its endpoints match its net debit.
    let value = entry();
    value.validate().unwrap();
    let restored: ItemLedgerEntry =
        serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(restored, value);
    assert_eq!(restored.delta.count_delta, -6);
    assert_eq!(restored.operation.as_str(), "consume");
}

#[test]
fn implicit_consume_retains_use_receipt_scope_in_persisted_payload() {
    use nightfall_api::domain::zone::phase4_projection::{ItemOperation, ItemReceiptRef};
    let mut value = entry();
    let reference = ItemReceiptRef {
        operation: ItemOperation::Use,
        key: uuid::Uuid::from_u128(7),
    };
    value.receipt_ref = Some(reference.key);
    value.delta.receipt = Some(reference);
    value.delta.causes = vec![
        cause("item_cast_payment", 0, -2, Some(reference.key)),
        cause("item_cast_payment", 2, -4, Some(reference.key)),
    ];
    value.validate().unwrap();
    let restored: ItemLedgerDelta =
        serde_json::from_value(serde_json::to_value(&value.delta).unwrap()).unwrap();
    assert_eq!(restored.receipt, Some(reference));
    value.delta.receipt.as_mut().unwrap().operation = ItemOperation::Consume;
    assert!(value.validate().is_err());
    value.delta.receipt = None;
    assert!(value.validate().is_err());
}
#[test]
fn invalid_bounds_fabricated_rows_identity_and_batch_ordinal_are_rejected() {
    let base = entry();
    let mut bad = base.clone();
    bad.tick = Tick(u64::MAX);
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.epoch = u64::MAX;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.zone_id = ZoneId(u32::MAX);
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.operation_ordinal = 1;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.delta.before = None;
    bad.delta.after = None;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.delta.after.as_mut().unwrap().id = ItemInstanceId::new(2).unwrap();
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.delta.after.as_mut().unwrap().revision = 3;
    assert!(bad.validate().is_err());
    let mut overlap = base.clone();
    overlap.delta.after.as_mut().unwrap().count = 9;
    assert!(overlap.validate().is_err());
    let mut high = base;
    high.delta.after.as_mut().unwrap().revision = i64::MAX.unsigned_abs();
    high.revision = i64::MAX.unsigned_abs();
    high.tick = Tick(i64::MAX.unsigned_abs());
    high.validate().unwrap();
}
