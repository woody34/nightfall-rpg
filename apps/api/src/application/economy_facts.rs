//! Ruling P5-D18: the API derives every design §11.5 economy outbox fact from the tick's ledger
//! rows, as Phase 4 `checkpoint.rs` derives item facts from ledger deltas. The zone emits none.
//!
//! One fact per **operation**: the rows of one tick that share an operation key (the receipt
//! `receipt:<scope>:<key>`, else the first automatic cause `<kind>:<resolution>`, design §11.3)
//! form one fact. The subject follows the receipt scope or the cause kind. The fact id is
//! `UUIDv5(NS_FACT, "fact:<zone>:<epoch>:<tick>:<operation key>")`, so a replayed checkpoint
//! derives the same id and `economy_counters` stays idempotent (B8). A row with neither a
//! receipt nor a cause derives no fact. Payloads are integers and strings only (E12).

use std::collections::BTreeMap;

use uuid::Uuid;

use super::economy::{EconomyError, OutboxFact, OwnerLedgerRow};

/// `UUIDv5` namespace of derived outbox fact ids (P5-D18).
pub const NS_FACT: Uuid = Uuid::from_u128(0x6e69_6768_7466_616c_6c2d_6661_6374_0000);

/// One ledger row as the derivation reads it, from either source: an economy row, or (P5-D22)
/// a character row of the V6 projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRow {
    /// `item_ledger.owner_kind`.
    pub owner_kind: String,
    /// The owner id.
    pub owner: Uuid,
    /// The instance.
    pub instance: u64,
    /// `item_ledger.op`.
    pub op: String,
    /// The T1 position.
    pub ordinal: u32,
    /// The template (of `after`, else `before`).
    pub template_id: u32,
    /// The signed count change of the row.
    pub count_delta: i64,
    /// `receipt:<scope>:<key>`, if the row carries one.
    pub receipt: Option<String>,
    /// The automatic causes, in order.
    pub causes: Vec<String>,
}

impl FactRow {
    /// An economy ledger row: `delta.{before, after, count_delta, receipt, causes}`.
    pub fn of_owner_row(row: &OwnerLedgerRow) -> Result<Self, EconomyError> {
        let scalar = |name: &str| row.delta.get(name).filter(|v| !v.is_null());
        let count = |name: &str| {
            scalar(name)
                .and_then(|v| v.get("count"))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0)
        };
        let template = scalar("after")
            .or_else(|| scalar("before"))
            .and_then(|v| v.get("template_id"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|t| u32::try_from(t).ok())
            .unwrap_or(0);
        let count_delta = match row
            .delta
            .get("count_delta")
            .and_then(serde_json::Value::as_i64)
        {
            Some(delta) => delta,
            None => count("after")
                .checked_sub(count("before"))
                .ok_or_else(|| EconomyError::Constraint("economy_fact_count".into()))?,
        };
        let text = |v: &serde_json::Value| v.as_str().map(str::to_owned);
        Ok(Self {
            owner_kind: row.owner_kind.clone(),
            owner: row.owner_id,
            instance: row.instance_id,
            op: row.op.clone(),
            ordinal: row.operation_ordinal,
            template_id: template,
            count_delta,
            receipt: row.delta.get("receipt").and_then(text),
            causes: row
                .delta
                .get("causes")
                .and_then(serde_json::Value::as_array)
                .map(|causes| causes.iter().filter_map(text).collect())
                .unwrap_or_default(),
        })
    }

    /// The operation key: the receipt, else the first cause.
    fn key(&self) -> Option<&str> {
        self.receipt
            .as_deref()
            .or_else(|| self.causes.first().map(String::as_str))
    }
}

/// The §11.5 subject of an operation key (receipt scope or cause kind, design §11.3).
#[must_use]
pub fn subject_of(key: &str) -> Option<&'static str> {
    let mut parts = key.splitn(3, ':');
    let kind = parts.next()?;
    let scope = if kind == "receipt" {
        parts.next()?
    } else {
        kind
    };
    Some(match scope {
        "loot" => "looted",
        "pk_drop" => "dropped_pk",
        "pick_up_item" | "herb" => "picked_up",
        "decay" => "decayed",
        "sweep" => "swept",
        "harvest" => "harvested",
        "vendor_buy" => "purchased",
        "vendor_sell" => "sold",
        "vendor_refund" => "refunded",
        "multisell_choose" => "exchanged",
        "craft_item" | "private_manufacture" => "crafted",
        "trade_confirm" | "trade" | "store_buy" | "store_sell" => "traded",
        "warehouse_deposit" => "deposited",
        "warehouse_withdraw" | "freight_withdraw" => "withdrawn",
        "freight_send" => "freighted",
        "mail_send" => "mailed",
        "mail_take" => "mail_taken",
        "mail_reject" | "mail_cancel" => "mail_returned",
        "mail_expiry" => "mail_expired",
        "delivery" => "delivered",
        "auction_bid" => "auction_bid",
        "auction_cancel_bid" => "auction_refunded",
        "auction_win" => "auction_awarded",
        "manor_buy_seed" | "manor_sell_crop" => "manor_traded",
        "register_recipe" | "sow" | "clear_refund" | "auction_expired" | "epoch_close"
        | "epoch_void" => "destroyed",
        _ => return None,
    })
}

/// The facts of one tick, in first-appearance order of their operation keys. An operation key
/// with no §11.5 subject is refused: the zone emitted a cause the API cannot account for.
pub fn derive_facts(
    zone: u32,
    epoch: u64,
    tick: u64,
    rows: &[FactRow],
) -> Result<Vec<OutboxFact>, EconomyError> {
    let mut order: Vec<&str> = Vec::new();
    let mut groups: BTreeMap<&str, Vec<&FactRow>> = BTreeMap::new();
    for row in rows {
        let Some(key) = row.key() else { continue };
        if !groups.contains_key(key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(row);
    }
    let mut facts = Vec::with_capacity(order.len());
    for key in order {
        let subject = subject_of(key)
            .ok_or_else(|| EconomyError::Constraint(format!("economy_fact_unmapped:{key}")))?;
        let members = groups.get(key).map_or(&[][..], Vec::as_slice);
        let rows: Vec<serde_json::Value> = members
            .iter()
            .map(|r| {
                serde_json::json!({
                    "owner_kind": r.owner_kind,
                    "owner": r.owner.to_string(),
                    "instance": r.instance,
                    "op": r.op,
                    "ordinal": r.ordinal,
                    "template_id": r.template_id,
                    "count_delta": r.count_delta,
                })
            })
            .collect();
        facts.push(OutboxFact {
            subject: subject.to_owned(),
            fact_id: Uuid::new_v5(&NS_FACT, format!("fact:{zone}:{epoch}:{tick}:{key}").as_bytes()),
            payload: serde_json::json!({
                "zone": zone,
                "epoch": epoch,
                "tick": tick,
                "operation": key,
                "rows": rows,
            }),
        });
    }
    Ok(facts)
}

/// The facts of an economy write's own ledger rows.
pub fn derive_write_facts(
    write: &super::economy::EconomyCheckpointWrite,
) -> Result<Vec<OutboxFact>, EconomyError> {
    let rows = write
        .ledger
        .iter()
        .map(FactRow::of_owner_row)
        .collect::<Result<Vec<_>, _>>()?;
    derive_facts(write.zone_id, write.epoch, write.tick, &rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn row(owner_kind: &str, op: &str, instance: u64, delta: serde_json::Value) -> OwnerLedgerRow {
        OwnerLedgerRow {
            owner_kind: owner_kind.into(),
            owner_id: Uuid::from_u128(7),
            zone_id: 1,
            epoch: 2,
            tick: 30,
            revision: 1,
            op: op.into(),
            instance_id: instance,
            operation_ordinal: 0,
            delta,
            receipt_ref: None,
        }
    }

    fn item(template: u32, count: u64) -> serde_json::Value {
        serde_json::json!({"template_id": template, "count": count})
    }

    #[test]
    fn a_tick_of_loot_pickup_split_and_decay_derives_one_fact_per_operation() {
        let pick = "receipt:pick_up_item:0c1b2a3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
        let split = "receipt:pick_up_item:11111111-2222-4333-8444-555555555555";
        let write = super::super::economy::EconomyCheckpointWrite {
            zone_id: 1,
            epoch: 2,
            tick: 30,
            ledger: vec![
                row(
                    "ground",
                    "loot",
                    10,
                    serde_json::json!({"before": null, "after": item(57, 90), "causes": ["loot:npc@29"]}),
                ),
                row(
                    "ground",
                    "transfer_out",
                    11,
                    serde_json::json!({"before": item(1864, 2), "after": null, "receipt": pick}),
                ),
                row(
                    "character",
                    "transfer_in",
                    11,
                    serde_json::json!({"before": null, "after": item(1864, 2), "receipt": pick}),
                ),
                row(
                    "ground",
                    "destroy",
                    12,
                    serde_json::json!({"before": item(57, 10), "after": null, "receipt": split, "causes": ["party_split:x"]}),
                ),
                row(
                    "character",
                    "party_split",
                    13,
                    serde_json::json!({"before": null, "after": item(57, 5), "count_delta": 5, "receipt": split}),
                ),
                row(
                    "ground",
                    "destroy",
                    14,
                    serde_json::json!({"before": item(57, 1), "after": null, "causes": ["decay:30"]}),
                ),
                row(
                    "warehouse",
                    "transfer_in",
                    15,
                    serde_json::json!({"before": null, "after": item(57, 1)}),
                ),
            ],
            ..Default::default()
        };
        let facts = derive_write_facts(&write).unwrap();
        let subjects: Vec<&str> = facts.iter().map(|f| f.subject.as_str()).collect();
        assert_eq!(
            subjects,
            vec!["looted", "picked_up", "picked_up", "decayed"],
            "the uncaused row derives none"
        );
        assert_eq!(
            facts[1].payload["rows"].as_array().unwrap().len(),
            2,
            "both halves of the pickup"
        );
        assert_eq!(facts[2].payload["rows"][1]["count_delta"], 5);
        assert_eq!(facts[1].payload["rows"][0]["count_delta"], -2);
        // The same tick derives the same ids (B8 idempotence); another tick, other ids.
        assert_eq!(derive_write_facts(&write).unwrap(), facts);
        let mut later = write.clone();
        later.tick = 31;
        assert_ne!(derive_write_facts(&later).unwrap()[0].fact_id, facts[0].fact_id);
        assert!(facts
            .iter()
            .all(|f| super::super::economy::OUTBOX_SUBJECTS.contains(&f.subject.as_str())));
    }

    #[test]
    fn every_operation_key_of_the_ledger_table_has_a_subject_and_unknown_keys_refuse() {
        for (key, subject) in [
            ("receipt:vendor_buy:k", "purchased"),
            ("receipt:vendor_sell:k", "sold"),
            ("receipt:warehouse_deposit:k", "deposited"),
            ("receipt:freight_withdraw:k", "withdrawn"),
            ("mail_expiry:m", "mail_expired"),
            ("delivery:d", "delivered"),
            ("auction_win:4", "auction_awarded"),
            ("epoch_void:3", "destroyed"),
            ("pk_drop:r", "dropped_pk"),
            ("sweep:c", "swept"),
        ] {
            assert_eq!(subject_of(key), Some(subject), "{key}");
        }
        assert_eq!(subject_of("receipt:mail_mark_read:k"), None, "mark read moves no item");
        let bad = FactRow {
            owner_kind: "ground".into(),
            owner: Uuid::nil(),
            instance: 1,
            op: "destroy".into(),
            ordinal: 0,
            template_id: 57,
            count_delta: -1,
            receipt: None,
            causes: vec!["mystery:1".into()],
        };
        assert!(
            matches!(derive_facts(1, 1, 1, &[bad]), Err(EconomyError::Constraint(name)) if name == "economy_fact_unmapped:mystery:1")
        );
    }
}
