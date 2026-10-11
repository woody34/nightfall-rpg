//! Raw, recursive version validation before prost/serde can discard extension presence.
//! The descriptor below freezes the legacy wire grammar, including nested old-tag payloads.

use super::record::CodecError;

pub(super) fn legacy_json(value: &serde_json::Value) -> Result<(), CodecError> {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, child) in fields {
                if matches!(
                    key.as_str(),
                    "phase3"
                        | "phase3_admission_policy"
                        | "active_effects"
                        | "pending_cast"
                        | "cp_damage"
                        | "cp_absorbed"
                        | "karma"
                        | "pk_count"
                        | "pvp_count"
                        | "flag_until"
                        | "flag_until_tick"
                        | "death_penalty_level"
                        | "resource_fractions"
                        | "reuse_remainders"
                        | "effect_remainders"
                        | "modifier_install_order"
                        | "baseline"
                        | "baseline_tick"
                        | "sp_reward"
                ) {
                    return Err(CodecError(format!("Phase 3 field {key} in legacy payload")));
                }
                legacy_json(child)?;
            }
        },
        serde_json::Value::Array(items) => {
            for item in items {
                legacy_json(item)?;
            }
        },
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {},
    }
    Ok(())
}

pub(super) fn record(bytes: &[u8], schema: u32, digested: bool) -> Result<(), CodecError> {
    message(bytes, "PbRecord", schema, digested)
}

fn varint(bytes: &mut &[u8]) -> Result<u64, CodecError> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = bytes
            .split_first()
            .ok_or_else(|| CodecError("truncated varint".into()))?;
        *bytes = rest;
        if shift == 63 && byte > 1 {
            return Err(CodecError("overflowed varint".into()));
        }
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err(CodecError("overflowed varint".into()))
}

fn message(mut bytes: &[u8], node: &str, schema: u32, digested: bool) -> Result<(), CodecError> {
    while !bytes.is_empty() {
        let key = varint(&mut bytes)?;
        let tag = u32::try_from(key >> 3).map_err(|e| CodecError(e.to_string()))?;
        let wire = key & 7;
        let (expected, mut child, json) = if schema >= 7 {
            phase3_field(node, tag).map_or_else(|| field(node, tag), Ok)?
        } else {
            field(node, tag)?
        };
        if wire != expected && !(expected == 3 && (wire == 0 || wire == 2)) {
            return Err(CodecError(format!("wrong wire type for {node}.{tag}")));
        }
        if (schema < 5
            && ((node == "PbCommand" && tag == 15)
                || (node == "PbPlayerLoad" && tag == 8)
                || (node == "PbOutput" && tag == 18)))
            || (schema < 6 && node == "PbOutput" && tag == 19)
        {
            return Err(CodecError(format!("{node}.{tag} is newer than record {schema}")));
        }
        if node == "PbRecord" && tag == 9 && !digested {
            child = Some("PbOutputs");
        }
        if node == "PbPlayerOutput" && tag == 2 {
            child = Some("PbOutputs");
        }
        if wire == 0 {
            let value = varint(&mut bytes)?;
            if schema < 7 && node == "PbDisposition" && tag == 5 && value >= 21 {
                return Err(CodecError("Phase 3 rejection in legacy record".into()));
            }
            if node == "PbAcceptResurrect" && tag == 3 && value > 1 {
                return Err(CodecError("invalid AcceptResurrect boolean".into()));
            }
        } else {
            let len =
                usize::try_from(varint(&mut bytes)?).map_err(|e| CodecError(e.to_string()))?;
            let (payload, rest) = bytes
                .split_at_checked(len)
                .ok_or_else(|| CodecError("truncated nested payload".into()))?;
            bytes = rest;
            if let Some(child) = child {
                message(payload, child, schema, digested)?;
            }
            if json {
                let value =
                    serde_json::from_slice(payload).map_err(|e| CodecError(e.to_string()))?;
                legacy_json(&value)?;
            }
        }
    }
    Ok(())
}

// Third tuple member identifies opaque protobuf bytes whose actual schema is JSON.
#[allow(clippy::match_same_arms, clippy::too_many_lines)]
fn field(node: &str, tag: u32) -> Result<(u64, Option<&'static str>, bool), CodecError> {
    Ok(match (node, tag) {
        ("PbRecord", 1) => (0, None, false),
        ("PbRecord", 2) => (0, None, false),
        ("PbRecord", 3) => (0, None, false),
        ("PbRecord", 4) => (0, None, false),
        ("PbRecord", 5) => (2, Some("PbCommand"), false),
        ("PbRecord", 6) => (2, Some("PbDisposition"), false),
        ("PbRecord", 7) => (2, Some("PbPlayerOutput"), false),
        ("PbRecord", 8) => (0, None, false),
        ("PbRecord", 9) => (2, None, false),
        ("PbRecord", 10) => (2, None, false),
        ("PbRecord", 11) => (0, None, false),
        ("PbSession", 1) => (2, None, false),
        ("PbSession", 2) => (0, None, false),
        ("PbVec", 1) => (0, None, false),
        ("PbVec", 2) => (0, None, false),
        ("PbCommand", 1) => (0, None, false),
        ("PbCommand", 2) => (2, Some("PbSession"), false),
        ("PbCommand", 3) => (0, None, false),
        ("PbCommand", 4..=15) => return field("PbCommandKind", tag),
        ("PbCommandKind", 15) => (2, None, true),
        ("PbCommandKind", 4) => (2, Some("PbSpawnPlayer"), false),
        ("PbCommandKind", 5) => (2, Some("PbSpawnNpc"), false),
        ("PbCommandKind", 6) => (2, Some("PbEntityRef"), false),
        ("PbCommandKind", 7) => (2, Some("PbReplace"), false),
        ("PbCommandKind", 8) => (2, Some("PbMoveTo"), false),
        ("PbCommandKind", 9) => (2, Some("PbEntityRef"), false),
        ("PbCommandKind", 10) => (2, Some("PbSetTarget"), false),
        ("PbCommandKind", 11) => (2, Some("PbEntityRef"), false),
        ("PbCommandKind", 12) => (2, Some("PbEntityRef"), false),
        ("PbCommandKind", 13) => (2, Some("PbEntityRef"), false),
        ("PbCommandKind", 14) => (2, Some("PbAddAggro"), false),
        ("PbAddAggro", 1) => (2, None, false),
        ("PbAddAggro", 2) => (2, None, false),
        ("PbPlayerLoad", 8) => (2, None, true),
        ("PbPlayerLoad", 7) => (0, None, false),
        ("PbPlayerLoad", 1) => (2, None, false),
        ("PbPlayerLoad", 2) => (0, None, false),
        ("PbPlayerLoad", 3) => (0, None, false),
        ("PbPlayerLoad", 4) => (0, None, false),
        ("PbPlayerLoad", 5) => (0, None, false),
        ("PbPlayerLoad", 6) => (0, None, false),
        ("PbStats", 1) => (0, None, false),
        ("PbStats", 2) => (0, None, false),
        ("PbStats", 3) => (0, None, false),
        ("PbStats", 4) => (0, None, false),
        ("PbStats", 5) => (0, None, false),
        ("PbStats", 6) => (0, None, false),
        ("PbStats", 7) => (0, None, false),
        ("PbStats", 8) => (0, None, false),
        ("PbStats", 9) => (0, None, false),
        ("PbStats", 10) => (0, None, false),
        ("PbNpcCombat", 1) => (2, None, false),
        ("PbNpcCombat", 2) => (2, Some("PbStats"), false),
        ("PbNpcCombat", 3) => (0, None, false),
        ("PbNpcCombat", 4) => (0, None, false),
        ("PbNpcCombat", 5) => (0, None, false),
        ("PbSwing", 1) => (2, None, false),
        ("PbSwing", 2) => (0, None, false),
        ("PbSwing", 3) => (0, None, false),
        ("PbSwing", 4) => (0, None, false),
        ("PbSwing", 5) => (0, None, false),
        ("PbCombatView", 1) => (2, None, false),
        ("PbCombatView", 2) => (0, None, false),
        ("PbCombatView", 3) => (0, None, false),
        ("PbCombatView", 4) => (0, None, false),
        ("PbCombatView", 5) => (0, None, false),
        ("PbCombatView", 6) => (0, None, false),
        ("PbCombatView", 7) => (0, None, false),
        ("PbCombatView", 8) => (2, Some("PbSwing"), false),
        ("PbAttackStarted", 1) => (2, None, false),
        ("PbAttackStarted", 2) => (2, None, false),
        ("PbAttackStarted", 3) => (0, None, false),
        ("PbAttackStarted", 4) => (0, None, false),
        ("PbAttackStarted", 5) => (0, None, false),
        ("PbAttackStarted", 6) => (0, None, false),
        ("PbAttackCancelled", 1) => (2, None, false),
        ("PbAttackCancelled", 2) => (2, None, false),
        ("PbAttackCancelled", 3) => (0, None, false),
        ("PbAttackCancelled", 4) => (0, None, false),
        ("PbHateChanged", 1) => (2, None, false),
        ("PbHateChanged", 2) => (2, None, false),
        ("PbHateChanged", 3) => (0, None, false),
        ("PbHateChanged", 4) => (0, None, false),
        ("PbHateChanged", 5) => (0, None, false),
        ("PbNpcIntentionChanged", 1) => (2, None, false),
        ("PbNpcIntentionChanged", 2) => (0, None, false),
        ("PbNpcIntentionChanged", 3) => (0, None, false),
        ("PbNpcIntentionChanged", 4) => (0, None, false),
        ("PbSpawnPlayer", 1) => (2, None, false),
        ("PbSpawnPlayer", 2) => (2, None, false),
        ("PbSpawnPlayer", 3) => (2, Some("PbVec"), false),
        ("PbSpawnPlayer", 4) => (0, None, false),
        ("PbSpawnPlayer", 5) => (0, None, false),
        ("PbSpawnPlayer", 6) => (2, Some("PbPlayerLoad"), false),
        ("PbSpawnNpc", 1) => (2, None, false),
        ("PbSpawnNpc", 2) => (2, Some("PbVec"), false),
        ("PbSpawnNpc", 3) => (0, None, false),
        ("PbSpawnNpc", 4) => (2, Some("PbNpcCombat"), false),
        ("PbEntityRef", 1) => (2, None, false),
        ("PbReplace", 1) => (2, None, false),
        ("PbReplace", 2) => (0, None, false),
        ("PbMoveTo", 1) => (2, None, false),
        ("PbMoveTo", 2) => (2, Some("PbVec"), false),
        ("PbDisposition", 1) => (0, None, false),
        ("PbDisposition", 2) => (2, Some("PbSession"), false),
        ("PbDisposition", 3) => (0, None, false),
        ("PbDisposition", 4) => (0, None, false),
        ("PbDisposition", 5) => (0, None, false),
        ("PbPlayerOutput", 1) => (2, None, false),
        ("PbPlayerOutput", 2) => (2, None, false),
        ("PbPlayerOutput", 3) => (2, None, false),
        ("PbOutputs", 1) => (2, Some("PbOutput"), false),
        ("PbOutput", 1..=19) => return field("PbOutputItem", tag),
        ("PbOutputItem", 18) => (2, None, true),
        ("PbOutputItem", 19) => (2, None, true),
        ("PbOutputItem", 1) => (2, Some("PbSpawnEvent"), false),
        ("PbOutputItem", 2) => (2, Some("PbMoveEvent"), false),
        ("PbOutputItem", 3) => (2, Some("PbDespawnEvent"), false),
        ("PbOutputItem", 4) => (2, Some("PbDisposition"), false),
        ("PbOutputItem", 5) => (2, Some("PbAccepted"), false),
        ("PbOutputItem", 6) => (2, Some("PbAttackResult"), false),
        ("PbOutputItem", 7) => (2, Some("PbEntityDied"), false),
        ("PbOutputItem", 8) => (2, Some("PbEntityRespawned"), false),
        ("PbOutputItem", 9) => (2, Some("PbStatsChanged"), false),
        ("PbOutputItem", 10) => (2, Some("PbXpGained"), false),
        ("PbOutputItem", 11) => (2, Some("PbLevelUp"), false),
        ("PbOutputItem", 12) => (2, Some("PbTargetChanged"), false),
        ("PbOutputItem", 13) => (2, Some("PbAttackStarted"), false),
        ("PbOutputItem", 14) => (2, Some("PbAttackCancelled"), false),
        ("PbOutputItem", 15) => (2, Some("PbHateChanged"), false),
        ("PbOutputItem", 16) => (2, Some("PbNpcIntentionChanged"), false),
        ("PbOutputItem", 17) => (2, Some("PbProgression"), false),
        ("PbAccepted", 3) => (0, None, false),
        ("PbAccepted", 1) => (0, None, false),
        ("PbAccepted", 2) => (0, None, false),
        ("PbSpawnEvent", 1) => (0, None, false),
        ("PbSpawnEvent", 2) => (2, None, false),
        ("PbSpawnEvent", 3) => (0, None, false),
        ("PbSpawnEvent", 4) => (2, None, false),
        ("PbSpawnEvent", 5) => (2, Some("PbVec"), false),
        ("PbSpawnEvent", 6) => (2, Some("PbVec"), false),
        ("PbSpawnEvent", 7) => (0, None, false),
        ("PbSpawnEvent", 8) => (0, None, false),
        ("PbSpawnEvent", 9) => (2, Some("PbCombatView"), false),
        ("PbMoveEvent", 1) => (0, None, false),
        ("PbMoveEvent", 2) => (2, None, false),
        ("PbMoveEvent", 3) => (2, Some("PbVec"), false),
        ("PbMoveEvent", 4) => (2, Some("PbVec"), false),
        ("PbMoveEvent", 5) => (0, None, false),
        ("PbDespawnEvent", 1) => (0, None, false),
        ("PbDespawnEvent", 2) => (2, None, false),
        ("PbSetTarget", 1) => (2, None, false),
        ("PbSetTarget", 2) => (2, None, false),
        ("PbAttackResult", 1) => (2, None, false),
        ("PbAttackResult", 2) => (2, None, false),
        ("PbAttackResult", 3) => (0, None, false),
        ("PbAttackResult", 4) => (0, None, false),
        ("PbAttackResult", 5) => (0, None, false),
        ("PbAttackResult", 6) => (0, None, false),
        ("PbAttackResult", 7) => (0, None, false),
        ("PbEntityDied", 1) => (2, None, false),
        ("PbEntityDied", 2) => (0, None, false),
        ("PbEntityDied", 3) => (2, None, false),
        ("PbEntityDied", 4) => (0, None, false),
        ("PbEntityRespawned", 1) => (2, None, false),
        ("PbEntityRespawned", 2) => (0, None, false),
        ("PbEntityRespawned", 3) => (2, Some("PbVec"), false),
        ("PbEntityRespawned", 4) => (0, None, false),
        ("PbEntityRespawned", 5) => (0, None, false),
        ("PbDeathFact", 1) => (2, None, false),
        ("PbDeathFact", 2) => (2, None, false),
        ("PbDeathFact", 3) => (0, None, false),
        ("PbProgression", 1) => (2, None, false),
        ("PbProgression", 2) => (0, None, false),
        ("PbProgression", 3) => (0, None, false),
        ("PbProgression", 4) => (0, None, false),
        ("PbProgression", 5) => (0, None, false),
        ("PbProgression", 6) => (0, None, false),
        ("PbProgression", 7) => (0, None, false),
        ("PbProgression", 8) => (0, None, false),
        ("PbProgression", 9) => (0, None, false),
        ("PbProgression", 10) => (2, Some("PbVec"), false),
        ("PbProgression", 11) => (0, None, false),
        ("PbProgression", 12) => (3, None, false),
        ("PbProgression", 13) => (2, Some("PbDeathFact"), false),
        ("PbProgression", 14) => (0, None, false),
        ("PbStatsChanged", 1) => (2, None, false),
        ("PbStatsChanged", 2) => (0, None, false),
        ("PbStatsChanged", 3) => (0, None, false),
        ("PbStatsChanged", 4) => (0, None, false),
        ("PbStatsChanged", 5) => (0, None, false),
        ("PbStatsChanged", 6) => (0, None, false),
        ("PbStatsChanged", 7) => (0, None, false),
        ("PbStatsChanged", 8) => (0, None, false),
        ("PbXpGained", 1) => (2, None, false),
        ("PbXpGained", 2) => (0, None, false),
        ("PbXpGained", 3) => (0, None, false),
        ("PbXpGained", 4) => (0, None, false),
        ("PbLevelUp", 1) => (2, None, false),
        ("PbLevelUp", 2) => (0, None, false),
        ("PbLevelUp", 3) => (0, None, false),
        ("PbTargetChanged", 1) => (2, None, false),
        ("PbTargetChanged", 2) => (2, None, false),
        ("PbTargetChanged", 3) => (0, None, false),
        _ => return Err(CodecError(format!("unknown legacy field {node}.{tag}"))),
    })
}

// New messages are traversed before prost's permissive boolean/unknown-field decoding.
// Old JSON tags keep legacy_json validation even inside record7: their grammar is frozen.
fn phase3_field(node: &str, tag: u32) -> Option<(u64, Option<&'static str>, bool)> {
    Some(match (node, tag) {
        ("PbCommand", 16) => (2, Some("PbCastSkill"), false),
        ("PbCommand", 17) => (2, Some("PbCancelCast"), false),
        ("PbCommand", 18) => (2, Some("PbAcceptResurrect"), false),
        ("PbCommand", 19) => (2, Some("PbLearnSkill"), false),
        ("PbPlayerLoad", 9)
        | ("PbOutput", 20 | 21 | 22 | 23 | 24 | 25 | 29 | 31)
        | ("PbCastSkill", 1 | 3)
        | ("PbCancelCast" | "PbAcceptResurrect", 1)
        | ("PbLearnSkill", 1..=3) => (2, None, false),
        ("PbCastSkill" | "PbCancelCast", 2)
        | ("PbAcceptResurrect", 2 | 3)
        | ("PbLearnSkill", 4 | 5) => (0, None, false),
        _ => return None,
    })
}
