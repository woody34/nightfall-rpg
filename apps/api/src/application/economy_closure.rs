//! Ruling P5-D28: an economy operation that touches more than one character settles in ONE
//! P5-D27 multi-character write, so its rows derive ONE fact (P5-D18; the fact id stays per
//! operation). This module is the fail-closed check that both character writers run before any
//! row is locked. A write is closed when every operation in it is complete inside the write:
//!
//! 1. **Receipts.** Every client-intent receipt (`receipt:<scope>:<key>`) that a character row
//!    names, in a V7 row or in an `economy` Consume contribution (P5-D26), is a receipt that this
//!    write claims. A row that names a receipt of another character's command is that command's
//!    other half (a party split recipient, a non-split looter, a crafter's fee).
//! 2. **Transfer pairs.** Every transfer link occurs in exactly one `transfer_out` and exactly one
//!    `transfer_in`, over the economy rows and the character rows of the write. A missing peer is
//!    in another character's write.
//! 3. **Party splits.** The `party_split` rows of a receipt add up to the count that the ground
//!    rows of the same receipt destroy, so every recipient is in the write. A `party_split` with an
//!    automatic cause only (a sweep to a party) has no ground row to count against: the
//!    single-character writer refuses it, and the multi-character writer takes it as given.
//!
//! The refusal is the permanent constraint `economy_multi_character_operation:<rule>:<key>`.

use std::collections::{BTreeMap, BTreeSet};

use super::economy::{CharacterCheckpointPart, EconomyCheckpointWrite, EconomyReceipt};
use super::economy_facts::FactRow;
use super::ports::{economy_cause, economy_text};
use super::CheckpointError;
use crate::domain::zone::phase4_projection::{ItemEvidenceValue, ItemOperation};

impl EconomyReceipt {
    /// The receipt text that the zone writes into the rows of this command (design §11.3).
    #[must_use]
    pub fn text(&self) -> String {
        format!("receipt:{}:{}", self.operation, self.request_key)
    }
}

fn refuse(rule: &str, key: &str) -> CheckpointError {
    CheckpointError::Constraint(format!("economy_multi_character_operation:{rule}:{key}"))
}

fn json_link(row: &super::economy::OwnerLedgerRow) -> Option<&str> {
    row.delta
        .get("transfer")
        .and_then(|t| t.get("link"))
        .and_then(serde_json::Value::as_str)
}

/// Refuse a write that carries only a part of an operation (P5-D28). `multi` is true for the
/// P5-D27 writer, which may carry a `party_split` with an automatic cause.
pub fn close_operations(
    parts: &[CharacterCheckpointPart<'_>],
    write: &EconomyCheckpointWrite,
    multi: bool,
) -> Result<(), CheckpointError> {
    let claimed: BTreeSet<String> = write.receipts.iter().map(EconomyReceipt::text).collect();
    // link -> (transfer_out rows, transfer_in rows)
    let mut links: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut count = |link: &str, out: bool| {
        let entry = links.entry(link.to_owned()).or_default();
        let slot = if out { &mut entry.0 } else { &mut entry.1 };
        *slot = slot.saturating_add(1);
    };
    // receipt -> the sum of its party_split count deltas
    let mut splits: BTreeMap<String, i64> = BTreeMap::new();
    for part in parts {
        let Some(work) = part.items else { continue };
        for delta in &work.projection.ledger {
            for cause in delta.causes.iter().filter(|c| c.family == "economy") {
                if let Some(ItemEvidenceValue::Text(receipt)) = cause.fields.get("receipt") {
                    if !claimed.contains(receipt) {
                        return Err(refuse("receipt", receipt));
                    }
                }
            }
            let link = economy_cause(delta).and_then(|c| economy_text(c, "link"));
            match (delta.operation, link) {
                (ItemOperation::TransferOut, Some(link)) => count(link, true),
                (ItemOperation::TransferIn, Some(link)) => count(link, false),
                _ => {},
            }
            if delta.operation == ItemOperation::PartySplit {
                let row = FactRow::of_plan(part.cp.character_id.as_uuid(), delta);
                match row.as_ref().and_then(|r| r.receipt.clone()) {
                    Some(receipt) => {
                        let sum = splits.entry(receipt).or_default();
                        *sum = sum.saturating_add(delta.count_delta);
                    },
                    None if multi => {},
                    None => {
                        let key = row
                            .and_then(|r| r.causes.first().cloned())
                            .unwrap_or_default();
                        return Err(refuse("party_split", &key));
                    },
                }
            }
        }
    }
    let mut destroyed: BTreeMap<String, i64> = BTreeMap::new();
    for row in &write.ledger {
        match (row.op.as_str(), json_link(row)) {
            ("transfer_out", Some(link)) => count(link, true),
            ("transfer_in", Some(link)) => count(link, false),
            _ => {},
        }
        if row.op == "destroy" {
            let fact = FactRow::of_owner_row(row).map_err(super::economy::checkpoint_error)?;
            if let Some(receipt) = fact.receipt.filter(|r| splits.contains_key(r)) {
                let sum = destroyed.entry(receipt).or_default();
                *sum = sum.saturating_sub(fact.count_delta);
            }
        }
    }
    if let Some((link, _)) = links.iter().find(|(_, pair)| **pair != (1, 1)) {
        return Err(refuse("transfer", link));
    }
    for (receipt, shares) in &splits {
        if destroyed.get(receipt).copied().unwrap_or(0) != *shares {
            return Err(refuse("party_split", receipt));
        }
    }
    Ok(())
}

/// The checkpoint fingerprint of a write with an economy part: the economy write joins it, so
/// an exact retry replays and a changed write under the same key is the D43 conflict. A write
/// without an economy part keeps its legacy fingerprint bytes.
pub fn economy_fingerprint(
    base: String,
    write: Option<&EconomyCheckpointWrite>,
) -> Result<String, CheckpointError> {
    use sha2::{Digest, Sha256};
    let Some(write) = write else { return Ok(base) };
    // `to_vec`, not a `Value`: lease owners are u128, which a JSON `Value` cannot hold.
    let mut hash = Sha256::new();
    hash.update(b"economy-checkpoint\0");
    hash.update(base.as_bytes());
    hash.update(b"\0");
    hash.update(serde_json::to_vec(write).map_err(anyhow::Error::from)?);
    Ok(format!("{:x}", hash.finalize()))
}
