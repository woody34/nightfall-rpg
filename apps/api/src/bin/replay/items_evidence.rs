//! Frozen D46 JSONL, released only after whole-recording byte/digest verification.
use anyhow::{bail, Context as _};
use nightfall_api::application::replay_log::{AppliedTickRecord, WatermarkReason};
use nightfall_api::domain::zone::evidence::{ItemEvidenceData, ItemEvidenceFact};
use nightfall_api::domain::zone::phase4_projection::{
    ItemAuditEvidence, ItemDeltaPlan, ItemEvidenceValue, ItemOperation, ItemReceiptRef,
};
use nightfall_api::domain::zone::{AppliedTickDraft, StateDigestVersion, ZoneEvent};
use nightfall_api::infrastructure::eventlog::Recording;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn line(out: &mut Vec<u8>, value: &impl Serialize) -> anyhow::Result<()> {
    let text = serde_json::to_vec(value)?;
    out.extend(quote_wide_integers(&text)?);
    out.push(b'\n');
    Ok(())
}

/// D80: every JSON integer outside ±2^53 is emitted as its exact decimal string. Works on
/// the canonical serialized text (no float round trip), skipping string contents.
#[allow(clippy::arithmetic_side_effects)] // Byte cursor bounded by the input length.
fn quote_wide_integers(text: &[u8]) -> anyhow::Result<Vec<u8>> {
    const LIMIT: u128 = 1 << 53;
    let mut out = Vec::with_capacity(text.len());
    let mut index = 0;
    let mut in_string = false;
    while let Some(&byte) = text.get(index) {
        if in_string {
            out.push(byte);
            if byte == b'\\' {
                out.push(*text.get(index + 1).context("truncated JSON escape")?);
                index += 2;
                continue;
            }
            in_string = byte != b'"';
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            out.push(byte);
            index += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while text.get(index).is_some_and(|b| {
                b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')
            }) {
                index += 1;
            }
            let token = text.get(start..index).context("JSON number")?;
            let digits = token.strip_prefix(b"-").unwrap_or(token);
            if !digits.iter().all(u8::is_ascii_digit) {
                bail!("non-integer evidence number");
            }
            let magnitude: u128 = std::str::from_utf8(digits)?.parse()?;
            if magnitude > LIMIT {
                out.push(b'"');
                out.extend_from_slice(token);
                out.push(b'"');
            } else {
                out.extend_from_slice(token);
            }
            continue;
        }
        out.push(byte);
        index += 1;
    }
    Ok(out)
}

/// D46 canonical instance scalars exclude D65 creation provenance (checkpoint data the API
/// verifies); granted instances carry it, so published rows drop it after every check
/// against the checkpoint has run.
fn strip_creation_origin(fact: &mut ItemEvidenceFact) {
    fn strip(value: &mut Value) {
        if let Value::Object(map) = value {
            map.remove("creation_origin");
            map.values_mut().for_each(strip);
        } else if let Value::Array(items) = value {
            items.iter_mut().for_each(strip);
        }
    }
    if let ItemEvidenceData::Delta(row) | ItemEvidenceData::Baseline(row) = &mut fact.fact {
        strip(row);
    }
}

#[derive(Serialize)]
struct SequencedFact<'a> {
    sequence: u64,
    #[serde(flatten)]
    fact: &'a nightfall_api::domain::zone::evidence::ItemEvidenceFact,
}

// D60 is a sidecar mapping only: canonical records retain their typed references.
fn receipt_text(value: Value) -> anyhow::Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let reference: ItemReceiptRef = serde_json::from_value(value)?;
    reference.validate()?;
    let operation = serde_json::to_value(reference.operation)?;
    Ok(json!(format!(
        "receipt:{}:{}",
        operation.as_str().context("receipt operation")?,
        reference.key
    )))
}

fn export_fact(mut fact: ItemEvidenceFact) -> anyhow::Result<ItemEvidenceFact> {
    match &mut fact.fact {
        ItemEvidenceData::Delta(value) => {
            let row = value.as_object_mut().context("item delta object")?;
            let receipt = receipt_text(row.remove("receipt").context("item delta receipt")?)?;
            if row.get("operation").and_then(Value::as_str) == Some("consume") {
                row.insert(
                    "receipt_refs".into(),
                    if receipt.is_null() {
                        json!([])
                    } else {
                        json!([receipt])
                    },
                );
            } else {
                row.insert("receipt_ref".into(), receipt);
            }
        },
        ItemEvidenceData::Formula(value) => {
            if let Some(operation) = value.get_mut("operation") {
                let operation = operation
                    .as_object_mut()
                    .context("formula operation object")?;
                let receipt =
                    receipt_text(operation.remove("receipt").context("formula receipt")?)?;
                operation.insert("receipt_ref".into(), receipt);
            }
        },
        ItemEvidenceData::Baseline(_) | ItemEvidenceData::Rng { .. } => {},
    }
    Ok(fact)
}

/// Fold real contributions at the verified tick boundary, retaining first resolution identity.
fn group_tick(facts: Vec<ItemEvidenceFact>) -> anyhow::Result<Vec<ItemEvidenceFact>> {
    let mut out: Vec<ItemEvidenceFact> = Vec::new();
    for fact in facts {
        let fact = export_fact(fact)?;
        if let ItemEvidenceData::Delta(next) = &fact.fact {
            if next.get("operation").and_then(Value::as_str) == Some("consume") {
                let previous = out.iter_mut().find(|previous| {
                    previous.entity == fact.entity
                        && previous.incarnation == fact.incarnation
                        && previous.origin == fact.origin
                        && previous.tick == fact.tick
                        && matches!(&previous.fact, ItemEvidenceData::Delta(row)
                            if row.get("operation").and_then(Value::as_str) == Some("consume")
                                && row.get("instance") == next.get("instance"))
                });
                if let Some(previous) = previous {
                    if let ItemEvidenceData::Delta(row) = &mut previous.fact {
                        let delta = row
                            .get("count_delta")
                            .and_then(Value::as_i64)
                            .and_then(|before| {
                                next.get("count_delta")
                                    .and_then(Value::as_i64)
                                    .and_then(|after| before.checked_add(after))
                            })
                            .context("consume sum overflow")?;
                        row["count_delta"] = json!(delta);
                        row["after"] = next.get("after").context("consume after")?.clone();
                        let refs = row
                            .get_mut("receipt_refs")
                            .and_then(Value::as_array_mut)
                            .context("consume receipt list")?;
                        for reference in next
                            .get("receipt_refs")
                            .and_then(Value::as_array)
                            .context("consume contribution receipt list")?
                        {
                            if !refs.contains(reference) {
                                refs.push(reference.clone());
                            }
                        }
                    }
                    continue;
                }
            }
        }
        out.push(fact);
    }
    Ok(out)
}

// D63 identities come from the verified durable causes, never inferred from receipts alone.
fn export_cause(cause: &ItemAuditEvidence) -> anyhow::Result<Value> {
    let Some(ItemEvidenceValue::Signed(count)) = cause.fields.get("count_delta") else {
        bail!("consume cause lacks signed count delta");
    };
    if *count >= 0 {
        bail!("consume cause is not a debit");
    }
    let identity = if let Some(receipt) = cause.contributing_receipt()? {
        receipt_text(serde_json::to_value(receipt)?)?
    } else if cause.family == "arrow_payment" {
        let Some(ItemEvidenceValue::Text(resolution)) = cause.fields.get("resolution_id") else {
            bail!("arrow payment lacks attack resolution identity");
        };
        if resolution.is_empty() || resolution.chars().any(char::is_whitespace) {
            bail!("invalid arrow resolution identity");
        }
        json!(format!("arrow:{resolution}"))
    } else if cause.family == "auto_shot_payment" {
        let Some(ItemEvidenceValue::Text(resolution)) = cause.fields.get("resolution_id") else {
            bail!("automatic shot payment lacks resolution identity");
        };
        if resolution.is_empty() || resolution.chars().any(char::is_whitespace) {
            bail!("invalid automatic shot resolution identity");
        }
        json!(format!("auto_shot:{resolution}"))
    } else if cause.family == "skill_cast_payment" {
        let Some(ItemEvidenceValue::Unsigned(id)) = cause.fields.get("cast_id") else {
            bail!("skill payment lacks cast identity");
        };
        // Keep the captured cast resolution namespace, including its actual numeric ID.
        json!(format!("item_cast:cast:{id}"))
    } else if cause.family == "skill_learn_payment" {
        // E3.4/D47 skill-book payment: the learn receipt key is the resolution identity.
        let Some(ItemEvidenceValue::Text(key)) = cause.fields.get("receipt") else {
            bail!("learn payment lacks its learn receipt");
        };
        let key: uuid::Uuid = key.parse().context("invalid learn receipt key")?;
        json!(format!("item_cast:learn:{key}"))
    } else {
        bail!("unsupported consume cause {}", cause.family);
    };
    Ok(json!({"cause":identity,"count_delta":count}))
}

fn attach_consume_causes(row: &mut Value, delta: &ItemDeltaPlan) -> anyhow::Result<()> {
    // Both sides are D66-normalized; ordinals differ only in namespace (the sidecar chain is
    // zero-based, the engine keeps tick-local logical ordinals).
    for (key, expected) in [
        ("instance", serde_json::to_value(delta.instance)?),
        ("before", serde_json::to_value(&delta.before)?),
        ("after", serde_json::to_value(&delta.after)?),
        ("count_delta", json!(delta.count_delta)),
    ] {
        if row.get(key) != Some(&expected) {
            bail!("observed consume {key} differs from verified checkpoint");
        }
    }
    if delta.operation != ItemOperation::Consume {
        bail!("consume checkpoint identity mismatch");
    }
    let causes = delta
        .causes
        .iter()
        .map(export_cause)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut refs = Vec::new();
    let mut unique = Vec::new();
    let mut total = 0_i64;
    for cause in &causes {
        let count = cause
            .get("count_delta")
            .and_then(Value::as_i64)
            .context("cause count")?;
        total = total.checked_add(count).context("cause count overflow")?;
        let identity = cause
            .get("cause")
            .and_then(Value::as_str)
            .context("cause identity")?;
        if identity.starts_with("receipt:") {
            let reference = json!(identity);
            if !unique.contains(&reference) {
                unique.push(reference.clone());
            }
            refs.push(reference);
        }
    }
    let canonical_refs = if delta.receipt_refs.is_empty() {
        delta.receipt.iter().copied().collect::<Vec<_>>()
    } else {
        delta.receipt_refs.clone()
    }
    .into_iter()
    .map(|reference| receipt_text(serde_json::to_value(reference)?))
    .collect::<anyhow::Result<Vec<_>>>()?;
    if causes.is_empty()
        || total != delta.count_delta
        || canonical_refs != unique
        || row.get("receipt_refs") != Some(&json!(unique))
    {
        bail!("consume causes do not match observed and durable contributions");
    }
    // Endpoint conservation is enforced on the D66-normalized chain, where any intervening
    // positive rows have been moved ahead of this aggregate.
    let first = causes
        .first()
        .and_then(|cause| cause.get("cause"))
        .and_then(Value::as_str)
        .context("first consume cause")?;
    if !first.starts_with("receipt:") {
        let (_, resolution) = first.split_once(':').context("automatic cause identity")?;
        if row.get("resolution_id").and_then(Value::as_str) != Some(resolution) {
            bail!("automatic consume resolution differs from verified cause");
        }
    }
    let row = row.as_object_mut().context("consume object")?;
    row.insert("receipt_refs".into(), json!(refs));
    row.insert("causes".into(), json!(causes));
    Ok(())
}

type ChainKey = (nightfall_api::domain::zone::EntityId, u32, bool, u64, u64);

fn chain_key(fact: &ItemEvidenceFact) -> Option<(ChainKey, &Value)> {
    let ItemEvidenceData::Delta(row) = &fact.fact else {
        return None;
    };
    let instance = row.get("instance").and_then(Value::as_u64)?;
    Some((
        (
            fact.entity,
            fact.incarnation,
            fact.origin == nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Offline,
            fact.tick.0,
            instance,
        ),
        row,
    ))
}

/// Observed tick-start (first before) and tick-end (last after) per instance chain.
fn chain_bounds(
    facts: &[ItemEvidenceFact],
) -> anyhow::Result<std::collections::BTreeMap<ChainKey, (Value, Value)>> {
    let mut bounds = std::collections::BTreeMap::new();
    for fact in facts {
        let Some((key, row)) = chain_key(fact) else {
            continue;
        };
        let before = row.get("before").context("delta before")?.clone();
        let after = row.get("after").context("delta after")?.clone();
        bounds
            .entry(key)
            .and_modify(|(_, end): &mut (Value, Value)| *end = after.clone())
            .or_insert((before, after));
    }
    Ok(bounds)
}

fn json_count(state: &Value) -> anyhow::Result<i128> {
    if state.is_null() {
        return Ok(0);
    }
    Ok(i128::from(
        state
            .get("count")
            .and_then(Value::as_u64)
            .context("canonical state count")?,
    ))
}

fn set(row: &mut Value, key: &str, value: Value) -> anyhow::Result<()> {
    row.as_object_mut()
        .context("canonical object")?
        .insert(key.into(), value);
    Ok(())
}

/// Canonical fields other than count/revision must survive a positive or Consume row (D67).
fn same_identity(left: &Value, right: &Value) -> bool {
    let strip = |value: &Value| {
        let mut value = value.clone();
        if let Some(object) = value.as_object_mut() {
            object.remove("count");
            object.remove("revision");
        }
        value
    };
    strip(left) == strip(right)
}

/// D66: per (owner, life, tick, instance) export every positive row first in cause order, then
/// the single Consume aggregate last. Rows are rebuilt from the observed tick-start state and
/// the actual net deltas only; the engine ledger and record bytes are untouched.
#[allow(clippy::too_many_lines)] // One chain pass: split, rebase, aggregate, terminal.
fn normalize_d66(
    facts: &mut [ItemEvidenceFact],
    bounds: &std::collections::BTreeMap<ChainKey, (Value, Value)>,
) -> anyhow::Result<()> {
    let mut chains: std::collections::BTreeMap<ChainKey, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (index, fact) in facts.iter().enumerate() {
        if let Some((key, _)) = chain_key(fact) {
            chains.entry(key).or_default().push(index);
        }
    }
    for (key, slots) in chains {
        let operation = |index: usize| -> anyhow::Result<String> {
            let Some((_, row)) = facts.get(index).and_then(chain_key) else {
                bail!("chain slot is not a delta");
            };
            Ok(row
                .get("operation")
                .and_then(Value::as_str)
                .context("delta operation")?
                .to_owned())
        };
        // Positive rows and count-neutral rows (e.g. an automatic arrow equip) keep their
        // observed order ahead of the aggregate, exactly as the engine ledger does.
        // D94: at most one terminal row (after = null, not the aggregate), the chain's last
        // observed row; it stays last, with the aggregate immediately before it.
        let mut positives = Vec::new();
        let mut consumes = Vec::new();
        let mut terminals = Vec::new();
        for &index in &slots {
            let ended = facts
                .get(index)
                .and_then(chain_key)
                .is_some_and(|(_, row)| row.get("after").is_some_and(Value::is_null));
            if operation(index)? == "consume" {
                consumes.push(index);
            } else if ended {
                terminals.push(index);
            } else {
                positives.push(index);
            }
        }
        let terminal = match terminals.as_slice() {
            [] => None,
            [index] if slots.last() == Some(index) => Some(*index),
            _ => bail!("D94 terminal row is not the chain's single last row"),
        };
        let [consume] = consumes.as_slice() else {
            if consumes.is_empty() {
                // D83: every chain carries the D76 row position, Consume or not.
                for (ordinal, &index) in slots.iter().enumerate() {
                    let fact = facts.get_mut(index).context("chain slot")?;
                    let ItemEvidenceData::Delta(row) = &mut fact.fact else {
                        bail!("chain slot is not a delta");
                    };
                    set(row, "operation_ordinal", json!(ordinal))?;
                }
                continue;
            }
            bail!("D66 chain has more than one Consume aggregate");
        };
        let (start, end) = bounds.get(&key).context("D66 chain bounds")?;
        let mut rows = Vec::with_capacity(slots.len());
        let mut state = start.clone();
        for (ordinal, &index) in positives.iter().enumerate() {
            let mut fact = facts.get(index).context("positive slot")?.clone();
            let ItemEvidenceData::Delta(row) = &mut fact.fact else {
                bail!("positive slot is not a delta");
            };
            // D100: non-terminal rows of either sign rebase in observed order.
            let delta = row
                .get("count_delta")
                .and_then(Value::as_i64)
                .context("D66 chain row count delta")?;
            let observed = row.get("after").context("non-terminal after")?;
            if observed.is_null()
                || (state.is_null() && delta <= 0)
                || (delta != 0 && !state.is_null() && !same_identity(&state, observed))
            {
                bail!("D66 non-terminal row changes instance identity");
            }
            let count = json_count(&state)?
                .checked_add(i128::from(delta))
                .and_then(|count| u64::try_from(count).ok())
                .context("D66 non-terminal row count is negative or overflows")?;
            let mut after = observed.clone();
            set(&mut after, "count", json!(count))?;
            set(row, "before", state.clone())?;
            set(row, "after", after.clone())?;
            set(row, "operation_ordinal", json!(ordinal))?;
            state = after;
            rows.push(fact);
        }
        let mut fact = facts.get(*consume).context("consume slot")?.clone();
        let ItemEvidenceData::Delta(row) = &mut fact.fact else {
            bail!("consume slot is not a delta");
        };
        let delta = row
            .get("count_delta")
            .and_then(Value::as_i64)
            .context("consume count delta")?;
        // With a terminal row the aggregate ends at its observed before-state (D94).
        let end = match terminal {
            Some(index) => {
                let Some((_, row)) = facts.get(index).and_then(chain_key) else {
                    bail!("terminal slot is not a delta");
                };
                row.get("before").context("terminal before")?.clone()
            },
            None => end.clone(),
        };
        if state.is_null()
            || json_count(&end)?.checked_sub(json_count(&state)?) != Some(i128::from(delta))
            || (!end.is_null() && !same_identity(&state, &end))
        {
            bail!("D66 Consume aggregate does not conserve the post-positive state");
        }
        set(row, "before", state)?;
        set(row, "after", end)?;
        set(row, "operation_ordinal", json!(positives.len()))?;
        rows.push(fact);
        if let Some(index) = terminal {
            let mut fact = facts.get(index).context("terminal slot")?.clone();
            let ItemEvidenceData::Delta(row) = &mut fact.fact else {
                bail!("terminal slot is not a delta");
            };
            set(
                row,
                "operation_ordinal",
                json!(positives.len().checked_add(1).context("ordinal overflow")?),
            )?;
            rows.push(fact);
        }
        // Rows are written back to the chain's slots in canonical order.
        let mut ordered = slots.clone();
        ordered.sort_unstable();
        for (slot, row) in ordered.iter().zip(rows) {
            *facts.get_mut(*slot).context("chain slot")? = row;
        }
    }
    Ok(())
}

/// D93/D96 use-row causes: automatic attack discharges name their attack resolution, cast
/// discharges name the cast's skill (from the recording's `SkillCastStarted` events), and a
/// receipt-bound use (manual shot charge) names its receipt. Every count delta is 0.
fn attach_use_causes(
    row: &mut Value,
    delta: &ItemDeltaPlan,
    casts: &std::collections::BTreeMap<u64, u32>,
) -> anyhow::Result<()> {
    for (key, expected) in [
        ("instance", serde_json::to_value(delta.instance)?),
        ("before", serde_json::to_value(&delta.before)?),
        ("after", serde_json::to_value(&delta.after)?),
        ("count_delta", json!(delta.count_delta)),
    ] {
        if row.get(key) != Some(&expected) {
            bail!("observed use {key} differs from verified checkpoint");
        }
    }
    if delta.operation != ItemOperation::Use || delta.count_delta != 0 {
        bail!("use checkpoint identity mismatch");
    }
    let mut causes = Vec::new();
    for cause in &delta.causes {
        if cause.family == "incarnation_reset" {
            // D142: the admission reset of a new incarnation clears the retained charges.
            let Some(ItemEvidenceValue::Text(resolution)) = cause.fields.get("resolution_id")
            else {
                bail!("incarnation reset lacks its resolution");
            };
            if resolution.is_empty() || resolution.chars().any(char::is_whitespace) {
                bail!("invalid incarnation reset resolution");
            }
            causes.push(
                json!({"cause": format!("incarnation_reset:{resolution}"), "count_delta": 0}),
            );
            continue;
        }
        if cause.family != "shot_discharge" {
            bail!("unsupported use cause {}", cause.family);
        }
        let identity = if let Some(ItemEvidenceValue::Unsigned(cast)) = cause.fields.get("cast") {
            let skill = casts
                .get(cast)
                .context("cast discharge without a recorded cast start")?;
            format!("cast_discharge:{skill}")
        } else {
            let Some(ItemEvidenceValue::Text(resolution)) = cause.fields.get("resolution_id")
            else {
                bail!("shot discharge lacks its attack resolution");
            };
            if resolution.is_empty() || resolution.chars().any(char::is_whitespace) {
                bail!("invalid shot discharge resolution");
            }
            format!("shot_discharge:{resolution}")
        };
        causes.push(json!({"cause": identity, "count_delta": 0}));
    }
    if let Some(receipt) = delta.receipt {
        causes.push(
            json!({"cause": receipt_text(serde_json::to_value(receipt)?)?, "count_delta": 0}),
        );
    }
    if causes.is_empty() {
        bail!("use row without a discharge or receipt cause");
    }
    let row = row.as_object_mut().context("use object")?;
    row.insert("causes".into(), json!(causes));
    Ok(())
}

type EntityKey = (nightfall_api::domain::zone::EntityId, u64);

fn export_tick(
    facts: Vec<ItemEvidenceFact>,
    events: &[ZoneEvent],
    casts: &std::collections::BTreeMap<u64, u32>,
) -> anyhow::Result<Vec<ItemEvidenceFact>> {
    let bounds = chain_bounds(&facts)?;
    let mut facts = group_tick(facts)?;
    normalize_d66(&mut facts, &bounds)?;
    // Use rows pair with the checkpoint's Use deltas of the same instance, in order.
    let mut used = std::collections::BTreeMap::<(EntityKey, u64), usize>::new();
    for fact in &mut facts {
        let ItemEvidenceData::Delta(row) = &mut fact.fact else {
            continue;
        };
        if row.get("operation").and_then(Value::as_str) == Some("use") {
            let instance = row
                .get("instance")
                .and_then(Value::as_u64)
                .context("use instance")?;
            let index = used
                .entry(((fact.entity, fact.tick.0), instance))
                .or_insert(0);
            let delta = events
                .iter()
                .filter_map(|event| {
                    if let ZoneEvent::ItemCheckpoint(projection) = event {
                        (projection.entity == fact.entity && projection.tick == fact.tick)
                            .then_some(projection)
                    } else {
                        None
                    }
                })
                .flat_map(|projection| &projection.ledger)
                .filter(|delta| {
                    delta.operation == ItemOperation::Use && delta.instance.get() == instance
                })
                .nth(*index)
                .context("use lacks a verified checkpoint row")?;
            *index = index.checked_add(1).context("use index overflow")?;
            attach_use_causes(row, delta, casts)?;
            continue;
        }
        if row.get("operation").and_then(Value::as_str) != Some("consume") {
            continue;
        }
        let mut matches =
            events
                .iter()
                .filter_map(|event| {
                    let ZoneEvent::ItemCheckpoint(projection) = event else {
                        return None;
                    };
                    (projection.entity == fact.entity && projection.tick == fact.tick &&
                projection.paused_core.is_some() == (fact.origin ==
                    nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Offline))
                .then_some(projection)
                })
                .flat_map(|projection| &projection.ledger)
                .filter(|delta| {
                    delta.operation == ItemOperation::Consume
                        && row.get("instance").and_then(Value::as_u64) == Some(delta.instance.get())
                });
        let delta = matches
            .next()
            .context("consume lacks verified checkpoint aggregate")?;
        if matches.next().is_some() {
            bail!("consume has duplicate checkpoint aggregates");
        }
        attach_consume_causes(row, delta)?;
    }
    Ok(facts)
}

#[allow(clippy::too_many_lines)] // Whole-recording validation precedes any artifact publication.
pub(super) fn collect(rec: &Recording, recording_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let seed = rec.snapshot.seed;
    let phase4 = rec
        .snapshot
        .phase4
        .as_ref()
        .context("item evidence requires V6 items")?;
    let records = u64::try_from(rec.records.len())?;
    let last_tick = rec
        .snapshot
        .tick
        .0
        .checked_add(records)
        .and_then(|n| n.checked_sub(1))
        .context("item evidence tick overflow")?;
    if rec.snapshot.meta.schema_version != 10
        || rec.snapshot.meta.digest_version != StateDigestVersion::BinaryV6
        || seed.zone.0 == 0
        || seed.epoch == 0
        || records == 0
        || rec.watermark.zone != seed.zone
        || rec.watermark.epoch != seed.epoch
        || rec.watermark.records != records
        || rec.watermark.last_tick.map(|tick| tick.0) != Some(last_tick)
        || rec.records.last().map(|record| record.tick.0) != Some(last_tick)
    {
        bail!("item evidence requires a complete, consistent V6 recording");
    }
    let reason = match rec.watermark.reason {
        WatermarkReason::Shutdown => "shutdown",
        WatermarkReason::EpochEnd => "epoch_closed",
        WatermarkReason::Capture => bail!("item evidence refuses partial/live captures"),
    };
    let mut rules_hash = String::with_capacity(64);
    for byte in phase4.rules_hash {
        use std::fmt::Write as _;
        write!(&mut rules_hash, "{byte:02x}")?;
    }
    let mut state = nightfall_api::application::replay_log::restore_snapshot(rec.snapshot.clone())?;
    state.enable_items_replay_evidence()?;
    let mut out = Vec::new();
    line(
        &mut out,
        &json!({"kind":"manifest","schema_version":1,"scope":"whole_recording",
        "recording_sha256":format!("{:x}",Sha256::digest(recording_bytes)),
        "zone":seed.zone.0,"epoch":seed.epoch,"snapshot_tick":rec.snapshot.tick,
        "watermark":{"zone":seed.zone.0,"epoch":seed.epoch,"records":records,"last_tick":last_tick,"reason":reason},
        "phase4_rules_hash":rules_hash,"snapshot_schema":10,"record_schema":8,"digest_version":"BinaryV6"}),
    )?;
    let mut sequence = 0_u64;
    let mut cursor = rec.snapshot.rng.word_pos;
    // Casts in flight at the boundary come from the snapshot sidecars; later starts from
    // SkillCastStarted events.
    let mut casts = std::collections::BTreeMap::<u64, u32>::new();
    if let Some(phase3) = rec.snapshot.phase3.as_ref() {
        for entity in phase3.entities.values() {
            if let nightfall_api::domain::zone::CastState::Casting(cast) = &entity.cast {
                casts.insert(cast.id.0, cast.skill.id.0);
            }
        }
    }
    for record in &rec.records {
        if record.zone != seed.zone
            || record.epoch != seed.epoch
            || record.digest_version != StateDigestVersion::BinaryV6
        {
            bail!("item evidence record policy mismatch at tick {}", record.tick.0);
        }
        let applied = state.run_tick(AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        })?;
        if !record.reproduced_by(&AppliedTickRecord::from_applied(seed.zone, &applied)) {
            bail!("item evidence replay divergence at tick {}", record.tick.0);
        }
        for event in &applied.events {
            if let ZoneEvent::SkillCastStarted { cast_id, skill, .. } = event {
                casts.insert(cast_id.0, skill.id.0);
            }
        }
        for fact in export_tick(state.take_items_replay_evidence(), &applied.events, &casts)? {
            if let nightfall_api::domain::zone::evidence::ItemEvidenceData::Rng {
                word_before,
                word_after,
                bound,
                value,
                ..
            } = &fact.fact
            {
                if word_after <= word_before
                    || *bound == 0
                    || value >= bound
                    || cursor != *word_before
                {
                    bail!("incomplete item RNG trace at tick {}", record.tick.0);
                }
                cursor = *word_after;
            }
            if (fact.incarnation == 0)
                != (fact.origin
                    == nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Offline)
                || fact.tick > record.tick
            {
                bail!("invalid item evidence subject/tick");
            }
            let mut fact = fact;
            strip_creation_origin(&mut fact);
            line(
                &mut out,
                &SequencedFact {
                    sequence,
                    fact: &fact,
                },
            )?;
            sequence = sequence
                .checked_add(1)
                .context("item evidence sequence overflow")?;
        }
        if cursor != state.replay_rng_word_position() {
            bail!("missing item RNG tail at tick {}", record.tick.0);
        }
    }
    if sequence == 0 {
        bail!("item evidence requires observed facts");
    }
    line(
        &mut out,
        &json!({"kind":"complete","facts":sequence,"ticks":records,"last_tick":last_tick}),
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(owner: u128, receipt: &Value, count: u64) -> ItemEvidenceFact {
        ItemEvidenceFact {
            entity: uuid::Uuid::from_u128(owner).into(),
            incarnation: 1,
            origin: nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Live,
            tick: nightfall_api::domain::zone::Tick(10),
            fact: ItemEvidenceData::Delta(json!({
                "operation":"consume", "instance": 1, "operation_ordinal":0,
                "before":{"count": count}, "after":{"count":count.saturating_sub(1)},
                "count_delta":-1, "receipt":receipt, "resolution_id":format!("resolution:{count}")
            })),
        }
    }

    #[test]
    fn consume_export_combines_real_contributions_and_scopes_the_ordered_receipts(
    ) -> anyhow::Result<()> {
        let first = json!({"operation":"enchant","key":uuid::Uuid::from_u128(1)});
        let second = json!({"operation":"enchant","key":uuid::Uuid::from_u128(2)});
        let original = vec![
            delta(1, &first, 4),
            delta(1, &second, 3),
            delta(1, &first, 2),
            delta(2, &second, 8),
        ];
        let saved = original.clone();
        let mapped = group_tick(original.clone())?;
        assert_eq!(original, saved);
        assert_eq!(mapped.len(), 2);
        let ItemEvidenceData::Delta(row) = &mapped.first().context("first delta")?.fact else {
            bail!("delta");
        };
        assert_eq!(row.get("before"), Some(&json!({"count":4})));
        assert_eq!(row.get("after"), Some(&json!({"count":1})));
        assert_eq!(row.get("count_delta"), Some(&json!(-3)));
        assert_eq!(row.get("resolution_id"), Some(&json!("resolution:4")));
        assert_eq!(
            row.get("receipt_refs"),
            Some(&json!([
                "receipt:enchant:00000000-0000-0000-0000-000000000001",
                "receipt:enchant:00000000-0000-0000-0000-000000000002"
            ]))
        );
        assert!(row.get("receipt").is_none());
        let mut fact = delta(1, &first, 2);
        if let ItemEvidenceData::Delta(row) = &mut fact.fact {
            row["operation"] = json!("enchant");
        }
        let mapped = export_fact(fact)?;
        let ItemEvidenceData::Delta(row) = mapped.fact else {
            bail!("delta");
        };
        assert_eq!(
            row.get("receipt_ref"),
            Some(&json!("receipt:enchant:00000000-0000-0000-0000-000000000001"))
        );
        assert!(row.get("receipt_refs").is_none());
        Ok(())
    }

    #[test]
    fn export_maps_formula_receipts_and_never_invents_automatic_cost_receipts() -> anyhow::Result<()>
    {
        let mut fact = delta(1, &Value::Null, 2);
        fact.fact = ItemEvidenceData::Formula(json!({"operation":{"receipt":{
            "operation":"enchant", "key":uuid::Uuid::from_u128(7)}, "target_before":null}}));
        let mapped = export_fact(fact)?;
        let ItemEvidenceData::Formula(row) = mapped.fact else {
            bail!("formula");
        };
        assert_eq!(
            row.pointer("/operation/receipt_ref"),
            Some(&json!("receipt:enchant:00000000-0000-0000-0000-000000000007"))
        );
        assert!(row.pointer("/operation/receipt").is_none());
        let mapped = export_fact(delta(1, &Value::Null, 2))?;
        let ItemEvidenceData::Delta(row) = mapped.fact else {
            bail!("delta");
        };
        assert_eq!(row.get("receipt_refs"), Some(&json!([])));
        assert!(export_fact(delta(
            1,
            &json!({"operation":"consume","key":uuid::Uuid::from_u128(7)}),
            2
        ))
        .is_err());
        Ok(())
    }

    fn consume_plan() -> anyhow::Result<ItemDeltaPlan> {
        let item = |count| {
            json!({"id":1,"template_id":3031,"count":count,
            "location":"inventory","slot":null,"enchant":0,"augment_id":null,
            "charges":0,"created_tick":0,"revision":1})
        };
        let receipt = ItemReceiptRef {
            operation: ItemOperation::Use,
            key: uuid::Uuid::from_u128(1),
        };
        let client = ItemAuditEvidence {
            family: "item_cast_payment".into(),
            fields: [
                ("receipt".into(), ItemEvidenceValue::Text(receipt.key.to_string())),
                ("count_delta".into(), ItemEvidenceValue::Signed(-1)),
            ]
            .into_iter()
            .collect(),
        };
        let automatic = ItemAuditEvidence {
            family: "skill_cast_payment".into(),
            fields: [
                ("cast_id".into(), ItemEvidenceValue::Unsigned(7)),
                ("count_delta".into(), ItemEvidenceValue::Signed(-2)),
            ]
            .into_iter()
            .collect(),
        };
        Ok(ItemDeltaPlan {
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance: nightfall_api::domain::zone::ItemInstanceId::new(1)?,
            before: Some(serde_json::from_value(item(5))?),
            after: Some(serde_json::from_value(item(1))?),
            count_delta: -4,
            receipt: Some(receipt),
            receipt_refs: vec![receipt],
            causes: vec![client.clone(), automatic, client],
        })
    }

    fn consume_observation(plan: &ItemDeltaPlan) -> anyhow::Result<Value> {
        Ok(json!({"operation":"consume","instance":plan.instance,"before":plan.before,
            "after":plan.after,"count_delta":plan.count_delta,"operation_ordinal":0,
            "receipt_refs":plan.receipt_refs.iter().map(|reference|
                receipt_text(serde_json::to_value(reference)?)).collect::<anyhow::Result<Vec<_>>>()?,
            "resolution_id":"resolution:1"}))
    }

    #[test]
    fn consume_causes_use_verified_order_preserve_duplicate_references_and_refuse_corruption(
    ) -> anyhow::Result<()> {
        let plan = consume_plan()?;
        plan.validate(1)?;
        let mut row = consume_observation(&plan)?;
        attach_consume_causes(&mut row, &plan)?;
        let reference = "receipt:use:00000000-0000-0000-0000-000000000001";
        assert_eq!(row.get("receipt_refs"), Some(&json!([reference, reference])));
        assert_eq!(
            row.get("causes"),
            Some(&json!([
            {"cause":reference,"count_delta":-1},
            {"cause":"item_cast:cast:7","count_delta":-2},
            {"cause":reference,"count_delta":-1}]))
        );
        for key in ["instance", "before", "after", "count_delta", "receipt_refs"] {
            let mut corrupt = consume_observation(&plan)?;
            corrupt
                .as_object_mut()
                .context("observation")?
                .insert(key.into(), Value::Null);
            assert!(attach_consume_causes(&mut corrupt, &plan).is_err(), "{key}");
        }
        let mut corrupt = plan.clone();
        corrupt.causes.clear();
        assert!(attach_consume_causes(&mut consume_observation(&corrupt)?, &corrupt).is_err());
        let mut corrupt = plan.clone();
        corrupt
            .causes
            .first_mut()
            .context("cause")?
            .fields
            .insert("count_delta".into(), ItemEvidenceValue::Signed(1));
        assert!(attach_consume_causes(&mut consume_observation(&corrupt)?, &corrupt).is_err());
        let mut corrupt = plan.clone();
        corrupt.causes.first_mut().context("cause")?.family = "unknown_payment".into();
        assert!(attach_consume_causes(&mut consume_observation(&corrupt)?, &corrupt).is_err());
        // Endpoint conservation moved to the D66 chain: causes attach, the chain refuses.
        let mut corrupt = plan;
        corrupt.after.as_mut().context("after")?.count = 4;
        let mut row = consume_observation(&corrupt)?;
        attach_consume_causes(&mut row, &corrupt)?;
        let mut fact = delta(1, &Value::Null, 5);
        fact.fact = ItemEvidenceData::Delta(row);
        let mut facts = vec![fact];
        let bounds = chain_bounds(&facts)?;
        assert!(normalize_d66(&mut facts, &bounds).is_err());
        Ok(())
    }

    #[test]
    fn consume_automatic_cost_has_no_receipt_and_requires_actual_cast_identity(
    ) -> anyhow::Result<()> {
        let mut plan = consume_plan()?;
        let automatic = plan.causes.get(1).context("automatic cause")?.clone();
        plan.causes = vec![automatic];
        plan.receipt = None;
        plan.receipt_refs.clear();
        plan.count_delta = -2;
        plan.after.as_mut().context("after")?.count = 3;
        let mut row = consume_observation(&plan)?;
        assert!(attach_consume_causes(&mut row, &plan).is_err());
        row.as_object_mut()
            .context("row")?
            .insert("resolution_id".into(), json!("cast:7"));
        attach_consume_causes(&mut row, &plan)?;
        assert_eq!(row.get("receipt_refs"), Some(&json!([])));
        assert_eq!(
            row.get("causes"),
            Some(&json!([{"cause":"item_cast:cast:7","count_delta":-2}]))
        );
        let mut fact = delta(1, &Value::Null, 5);
        fact.fact = ItemEvidenceData::Delta(row);
        // Restore the capture-side receipt field before exercising missing checkpoint refusal.
        if let ItemEvidenceData::Delta(row) = &mut fact.fact {
            let object = row.as_object_mut().context("row")?;
            object.remove("receipt_refs");
            object.remove("causes");
            object.insert("receipt".into(), Value::Null);
        }
        assert!(export_tick(vec![fact], &[], &std::collections::BTreeMap::new()).is_err());
        Ok(())
    }

    fn stack(count: u64, revision: u64) -> Value {
        json!({"id":1,"template_id":1835,"count":count,"location":"inventory","slot":null,
            "enchant":0,"augment_id":null,"charges":0,"created_tick":0,"revision":revision})
    }

    fn observed(operation: &str, before: Option<u64>, after: Option<u64>) -> ItemEvidenceFact {
        let state =
            |count: Option<u64>, revision| count.map_or(Value::Null, |c| stack(c, revision));
        let mut fact = delta(1, &Value::Null, 1);
        let change = i128::from(after.unwrap_or(0)).saturating_sub(i128::from(before.unwrap_or(0)));
        fact.fact = ItemEvidenceData::Delta(json!({
            "operation":operation, "instance":1, "operation_ordinal":0,
            "before":state(before, 1), "after":state(after, 2), "count_delta":change,
            "receipt":null, "resolution_id":format!("{operation}:{before:?}")
        }));
        fact
    }

    fn d66(observations: Vec<ItemEvidenceFact>) -> anyhow::Result<Vec<Value>> {
        let bounds = chain_bounds(&observations)?;
        let mut facts = group_tick(observations)?;
        normalize_d66(&mut facts, &bounds)?;
        let mut rows = Vec::new();
        for fact in facts {
            let ItemEvidenceData::Delta(row) = fact.fact else {
                bail!("delta");
            };
            rows.push(json!({
                "operation":row.get("operation"), "ordinal":row.get("operation_ordinal"),
                "before":row.pointer("/before/count"), "after":row.pointer("/after/count"),
                "delta":row.get("count_delta")}));
        }
        Ok(rows)
    }

    /// T10/B3: an epoch whose only item activity is an offline grant exports, both when an
    /// earlier tick drew non-item words (NPC spawn ids, now tabulated as zone draws) and when
    /// the whole epoch is draw-free; the gap-free cursor still holds across all facts.
    #[test]
    fn offline_grant_only_epochs_export_with_a_gap_free_cursor() -> anyhow::Result<()> {
        for (bytes, expect_zone_draws) in [
            (&include_bytes!("../../../fixtures/items/offline-grant-only.nfr")[..], true),
            (&include_bytes!("../../../fixtures/items/draw-free-grant.nfr")[..], false),
        ] {
            let recording = nightfall_api::infrastructure::eventlog::Recording::from_bytes(bytes)?;
            let out = collect(&recording, bytes)?;
            let rows: Vec<Value> = std::str::from_utf8(&out)?
                .lines()
                .map(serde_json::from_str)
                .collect::<Result<_, _>>()?;
            let mut cursor = recording.snapshot.rng.word_pos;
            let mut zone_draws = 0;
            for row in &rows {
                if row.get("kind").and_then(Value::as_str) == Some("item_rng") {
                    let data = row.get("data").context("rng data")?;
                    let word = |key: &str| -> anyhow::Result<u128> {
                        let value = data.get(key).context("rng word")?;
                        value
                            .as_u64()
                            .map(u128::from)
                            .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                            .context("rng word value")
                    };
                    assert_eq!(word("word_before")?, cursor);
                    cursor = word("word_after")?;
                    zone_draws += usize::from(
                        data.get("callsite").and_then(Value::as_str) == Some("entity_id"),
                    );
                }
            }
            assert_eq!(zone_draws > 0, expect_zone_draws);
            assert!(rows
                .iter()
                .any(|row| row.get("kind").and_then(Value::as_str) == Some("item_delta")));
        }
        Ok(())
    }

    #[test]
    fn d83_consume_free_chains_export_row_positions() -> anyhow::Result<()> {
        let logical = |operation: &str, before, after, ordinal: u32| {
            let mut fact = observed(operation, before, after);
            if let ItemEvidenceData::Delta(row) = &mut fact.fact {
                set(row, "operation_ordinal", json!(ordinal)).ok();
                set(row, "resolution_id", json!(format!("{operation}:{ordinal}"))).ok();
            }
            fact
        };
        // A lone Equip keeps no logical ordinal: position 0.
        let equip = d66(vec![logical("equip", Some(1), Some(1), 3)])?;
        assert_eq!(equip.first().and_then(|row| row.get("ordinal")), Some(&json!(0)));
        // Unequip then Destroy of one instance in one operation: positions 0 and 1.
        let removal = d66(vec![
            logical("unequip", Some(1), Some(1), 5),
            logical("destroy", Some(1), None, 5),
        ])?;
        assert_eq!(
            removal
                .iter()
                .map(|row| row.get("ordinal").cloned())
                .collect::<Vec<_>>(),
            vec![Some(json!(0)), Some(json!(1))]
        );
        Ok(())
    }

    #[test]
    fn d66_exports_positive_rows_first_then_the_final_consume_aggregate() -> anyhow::Result<()> {
        // 3 -> 2 -> 5 -> 4 exports grant(3->6, +3) then consume(6->4, -2).
        let interleaved = d66(vec![
            observed("consume", Some(3), Some(2)),
            observed("grant", Some(2), Some(5)),
            observed("consume", Some(5), Some(4)),
        ])?;
        assert_eq!(
            interleaved,
            vec![
                json!({"operation":"grant","ordinal":0,"before":3,"after":6,"delta":3}),
                json!({"operation":"consume","ordinal":1,"before":6,"after":4,"delta":-2}),
            ]
        );
        // Start 1, grant +3, consume 2 exports grant(1->4) then consume(4->2).
        let restock = d66(vec![
            observed("grant", Some(1), Some(4)),
            observed("consume", Some(4), Some(2)),
        ])?;
        assert_eq!(
            restock,
            vec![
                json!({"operation":"grant","ordinal":0,"before":1,"after":4,"delta":3}),
                json!({"operation":"consume","ordinal":1,"before":4,"after":2,"delta":-2}),
            ]
        );
        // Start absent, grant 3, consume 1 exports grant(None->3) then consume(3->2).
        let fresh = d66(vec![
            observed("grant", None, Some(3)),
            observed("consume", Some(3), Some(2)),
        ])?;
        assert_eq!(
            fresh,
            vec![
                json!({"operation":"grant","ordinal":0,"before":null,"after":3,"delta":3}),
                json!({"operation":"consume","ordinal":1,"before":3,"after":2,"delta":-1}),
            ]
        );
        // A plain aggregate keeps ordinal 0 and its own endpoints.
        let plain = d66(vec![
            observed("consume", Some(3), Some(2)),
            observed("consume", Some(2), None),
        ])?;
        assert_eq!(
            plain,
            vec![json!({"operation":"consume","ordinal":0,"before":3,"after":null,"delta":-3})]
        );
        Ok(())
    }

    #[test]
    fn d66_refuses_unconserved_or_unorderable_consume_chains() -> anyhow::Result<()> {
        let mut broken = observed("consume", Some(5), Some(4));
        if let ItemEvidenceData::Delta(row) = &mut broken.fact {
            row["count_delta"] = json!(-2);
        }
        assert!(d66(vec![observed("grant", Some(2), Some(5)), broken]).is_err());
        let mut equip = observed("equip", Some(3), Some(3));
        if let ItemEvidenceData::Delta(row) = &mut equip.fact {
            *row.pointer_mut("/after/location").context("location")? = json!("paperdoll");
        }
        // A count-neutral automatic equip stays ahead of the aggregate (bow arrow auto-select).
        let mut equipped = observed("consume", Some(3), Some(2));
        if let ItemEvidenceData::Delta(row) = &mut equipped.fact {
            *row.pointer_mut("/before/location").context("location")? = json!("paperdoll");
            *row.pointer_mut("/after/location").context("location")? = json!("paperdoll");
        }
        assert_eq!(
            d66(vec![equip, equipped])?,
            vec![
                json!({"operation":"equip","ordinal":0,"before":3,"after":3,"delta":0}),
                json!({"operation":"consume","ordinal":1,"before":3,"after":2,"delta":-1}),
            ]
        );
        // D100: a partial Destroy precedes the aggregate, rebased in observed order.
        assert_eq!(
            d66(vec![
                observed("destroy", Some(3), Some(2)),
                observed("consume", Some(2), Some(1))
            ])?,
            vec![
                json!({"operation":"destroy","ordinal":0,"before":3,"after":2,"delta":-1}),
                json!({"operation":"consume","ordinal":1,"before":2,"after":1,"delta":-1}),
            ]
        );
        // D94 [C, T]: the terminal Destroy stays last; the aggregate ends at its before.
        assert_eq!(
            d66(vec![
                observed("consume", Some(3), Some(2)),
                observed("destroy", Some(2), None)
            ])?,
            vec![
                json!({"operation":"consume","ordinal":0,"before":3,"after":2,"delta":-1}),
                json!({"operation":"destroy","ordinal":1,"before":2,"after":null,"delta":-2}),
            ]
        );
        // D94 [E/G, C, T]: Consume 3->2, Grant 2->5, Destroy 5->None rebases the grant first.
        assert_eq!(
            d66(vec![
                observed("consume", Some(3), Some(2)),
                observed("grant", Some(2), Some(5)),
                observed("destroy", Some(5), None)
            ])?,
            vec![
                json!({"operation":"grant","ordinal":0,"before":3,"after":6,"delta":3}),
                json!({"operation":"consume","ordinal":1,"before":6,"after":5,"delta":-1}),
                json!({"operation":"destroy","ordinal":2,"before":5,"after":null,"delta":-5}),
            ]
        );
        // A terminal row that is not last (or two of them) is refused.
        assert!(d66(vec![
            observed("destroy", Some(3), None),
            observed("consume", Some(3), Some(2))
        ])
        .is_err());
        let mut relabelled = observed("grant", Some(2), Some(5));
        if let ItemEvidenceData::Delta(row) = &mut relabelled.fact {
            *row.pointer_mut("/after/template_id").context("template")? = json!(1463);
        }
        assert!(d66(vec![
            observed("consume", Some(3), Some(2)),
            relabelled,
            observed("consume", Some(5), Some(4)),
        ])
        .is_err());
        Ok(())
    }

    #[test]
    fn item_export_requires_export_mode_and_distinct_fresh_destination() {
        for args in [
            vec![
                "check",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "proof.jsonl",
            ],
            vec![
                "export",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "record.nfr",
            ],
            vec![
                "export",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "proof.jsonl",
                "--evidence-out",
                "old.jsonl",
            ],
        ] {
            assert!(crate::parse_args(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(crate::parse_args(
            [
                "export",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "proof.jsonl"
            ]
            .into_iter()
            .map(str::to_owned)
        )
        .is_ok());
    }

    #[test]
    fn historical_recording_cannot_be_relabelled_as_item_evidence() -> anyhow::Result<()> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/sessions/phase3-gameplay-v5.nfr");
        let (recording, bytes) = Recording::read_with_bytes(&path)?;
        assert!(collect(&recording, &bytes).is_err());
        Ok(())
    }

    #[test]
    fn cursor_is_an_exact_json_integer_above_u64() -> anyhow::Result<()> {
        let before = u128::from(u64::MAX).saturating_add(1);
        let fact = ItemEvidenceFact {
            entity: "00000000-0000-0000-0000-000000000001".parse()?,
            incarnation: 1,
            origin: nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Live,
            tick: nightfall_api::domain::zone::Tick(1),
            fact: ItemEvidenceData::Rng {
                callsite: "enchant".into(),
                word_before: before,
                word_after: before.saturating_add(1),
                bound: 100_000,
                value: 1,
                resolution_id: "enchant:wide-cursor".into(),
            },
        };
        let mut out = Vec::new();
        line(
            &mut out,
            &SequencedFact {
                sequence: 0,
                fact: &fact,
            },
        )?;
        let text = String::from_utf8(out)?;
        // D80: integers outside ±2^53 are exact decimal strings.
        assert!(text.contains("\"word_before\":\"18446744073709551616\""));
        assert!(text.contains("\"word_after\":\"18446744073709551617\""));
        let mut small = Vec::new();
        line(
            &mut small,
            &json!({"a": 9_007_199_254_740_992_u64, "b": -9_007_199_254_740_993_i64, "c": "12345678901234567890", "d": [1, 2]}),
        )?;
        assert_eq!(
            String::from_utf8(small)?,
            "{\"a\":9007199254740992,\"b\":\"-9007199254740993\",\"c\":\"12345678901234567890\",\"d\":[1,2]}\n"
        );
        assert!(text.contains("\"kind\":\"item_rng\""));
        Ok(())
    }
}
