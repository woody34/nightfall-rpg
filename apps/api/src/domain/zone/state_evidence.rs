//! Replay-only instrumentation; enabling it cannot change simulation inputs or RNG draws.
use super::*;
use crate::domain::zone::effects::TargetBinding;
use crate::domain::zone::evidence::{EvidenceBuffer, EvidenceDraw, EvidenceFact};
use crate::domain::zone::HateEntry;
use serde_json::{json, Value};

impl ZoneState {
    /// Enable diagnostics on a replay instance. Nothing is added to snapshots or digests.
    pub fn enable_replay_evidence(&mut self) {
        self.evidence = self.phase3.as_ref().map(|_| EvidenceBuffer::default());
    }

    /// Drain observations only after the caller has verified this tick's reproduced record.
    pub fn take_replay_evidence(&mut self) -> Vec<EvidenceFact> {
        self.evidence.as_mut().map_or_else(Vec::new, |buffer| {
            buffer.draws.clear();
            std::mem::take(&mut buffer.facts)
        })
    }

    pub(super) fn evidence_fact(&mut self, kind: &str, entity: EntityId, data: Value) {
        if let Some(buffer) = &mut self.evidence {
            buffer.facts.push(EvidenceFact {
                kind: kind.to_owned(),
                entity,
                tick: self.next_tick,
                data,
            });
        }
    }

    pub(super) fn evidence_mark(&self) -> usize {
        self.evidence
            .as_ref()
            .map_or(0, |buffer| buffer.draws.len())
    }

    pub(super) fn evidence_draws(&self, start: usize) -> &[EvidenceDraw] {
        self.evidence
            .as_ref()
            .and_then(|buffer| buffer.draws.get(start..))
            .unwrap_or(&[])
    }

    pub(super) fn evidence_roll(
        &mut self,
        site: &str,
        target: Option<TargetBinding>,
        bound: u32,
    ) -> u32 {
        let before = self.rng.get_word_pos();
        let value = self.roll_below(bound);
        if self.items_evidence_enabled() {
            if let Some(subject) = target {
                self.item_evidence(
                    subject,
                    crate::domain::zone::evidence::ItemEvidenceData::Rng {
                        callsite: site.into(),
                        word_before: before,
                        word_after: self.rng.get_word_pos(),
                        bound,
                        value,
                        resolution_id: format!("damage:{before}"),
                    },
                );
            }
        }
        if let Some(buffer) = &mut self.evidence {
            let draw = EvidenceDraw {
                callsite: site.to_owned(),
                bound,
                value,
                target,
                word_before: before.to_string(),
                word_after: self.rng.get_word_pos().to_string(),
            };
            if let Some(target) = target {
                buffer.facts.push(EvidenceFact {
                    kind: "rng_draw".to_owned(),
                    entity: target.entity,
                    tick: self.next_tick,
                    data: json!(&draw),
                });
            }
            buffer.draws.push(draw);
        }
        value
    }

    pub(super) fn evidence_hate_removal(
        &mut self,
        npc: EntityId,
        source: EntityId,
        row: HateEntry,
        reason: &str,
    ) {
        if self.evidence.is_some() {
            self.evidence_fact(
                "hate_delta",
                npc,
                json!({
                    "source": source, "direction": "remove", "amount": row.hate,
                    "removed_row": row, "reason": reason
                }),
            );
        }
    }

    pub(super) fn evidence_context(&self, id: EntityId) -> Result<Value, TickError> {
        if self.evidence.is_none() {
            return Ok(Value::Null);
        }
        let effective = self.effective(id)?;
        Ok(json!({"entity": self.entities.get(&id), "phase3": self.sidecar(id)?,
            "effective": effective.as_ref(), "hate": self.hate.get(&id)}))
    }
}
