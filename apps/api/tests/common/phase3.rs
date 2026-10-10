//! Shared typed test doubles for learning use cases and transport. No real actor claims.
#![allow(missing_docs, unreachable_pub, clippy::unwrap_used)]

use super::api;
use api::application::ports::{
    learn_skill_fingerprint, FrozenLearnResult, LearnReceiptLookup, LearnReceiptRepository,
    LearnSkillReply, LearnableSkill, LearnableSkillsState, LearnedEffectiveStats, Phase3Runtime,
    SkillCatalogue, SkillCatalogueEntry, SkillCataloguePort,
};
use api::application::{AppError, IdempotencyKey};
use api::domain::class::{ClassId, SkillLearnDef};
use api::domain::zone::skill::{SkillId, SkillRef};
use api::domain::zone::Tick;
use api::domain::{AccountId, CharacterId, SessionGeneration};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default)]
pub struct State {
    pub receipts: BTreeMap<(AccountId, IdempotencyKey), (String, FrozenLearnResult)>,
    pub lookups: usize,
    pub generations: usize,
    pub enqueues: usize,
    pub writes: usize,
    pub events: usize,
    pub sp: u64,
    pub rank: u32,
    pub generation: Option<SessionGeneration>,
    pub owner: Option<AccountId>,
    pub character: Option<CharacterId>,
    pub receipt_failure: bool,
    pub failure: Option<&'static str>,
    pub stale_after_capture: bool,
    pub drop_reply: bool,
}

#[derive(Clone)]
pub struct Runtime(pub Arc<Mutex<State>>);
impl Runtime {
    pub fn new(owner: AccountId, character: CharacterId) -> Self {
        Self(Arc::new(Mutex::new(State {
            sp: 100,
            generation: Some(SessionGeneration::FIRST),
            owner: Some(owner),
            character: Some(character),
            ..Default::default()
        })))
    }
    fn check(
        state: &State,
        caller: AccountId,
        character: CharacterId,
        generation: SessionGeneration,
    ) -> Result<(), AppError> {
        if state.owner != Some(caller)
            || state.character != Some(character)
            || state.generation != Some(generation)
        {
            return Err(AppError::FailedPrecondition("stale generation".into()));
        }
        match state.failure {
            Some("unavailable") => Err(AppError::Unavailable("zone stopped".into())),
            Some("infrastructure") => {
                Err(AppError::Infrastructure(anyhow::anyhow!("private database failure")))
            },
            Some("resource_exhausted") => Err(AppError::ResourceExhausted("queue full".into())),
            Some(reason) => Err(AppError::FailedPrecondition(reason.into())),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl LearnReceiptRepository for Runtime {
    async fn learn_receipt_lookup(
        &self,
        caller: AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<LearnReceiptLookup> {
        let mut state = self.0.lock();
        state.lookups = state.lookups.checked_add(1).unwrap();
        anyhow::ensure!(!state.receipt_failure, "receipt database failure");
        Ok(match state.receipts.get(&(caller, *key)) {
            Some((stored, result)) if stored == fingerprint => {
                LearnReceiptLookup::Known(result.clone())
            },
            Some(_) => LearnReceiptLookup::Conflict,
            None => LearnReceiptLookup::Unknown,
        })
    }
}

#[async_trait::async_trait]
impl Phase3Runtime for Runtime {
    async fn generation(
        &self,
        _caller: AccountId,
        _character: CharacterId,
    ) -> Result<SessionGeneration, AppError> {
        let mut state = self.0.lock();
        state.generations = state.generations.checked_add(1).unwrap();
        let generation = state.generation.ok_or_else(|| {
            AppError::FailedPrecondition("character has no live zone session".into())
        })?;
        if state.stale_after_capture {
            state.generation = generation.next();
        }
        Ok(generation)
    }
    async fn learnable_skills(
        &self,
        caller: AccountId,
        character: CharacterId,
        generation: SessionGeneration,
    ) -> Result<LearnableSkillsState, AppError> {
        let state = self.0.lock();
        Self::check(&state, caller, character, generation)?;
        Ok(LearnableSkillsState {
            current_class_id: ClassId(0),
            sp: state.sp,
            skills: vec![LearnableSkill {
                skill: SkillLearnDef {
                    key: "l2.skill.3".into(),
                    skill_id: 3,
                    l2_ref: "Power Strike".into(),
                    learned_by_npc: true,
                    skill_level: state.rank.checked_add(1).unwrap(),
                    required_level: 1,
                    sp_cost: 10,
                    auto_get: false,
                    required_items: vec![],
                },
                effect_implemented: true,
                learned: false,
                affordable: state.sp >= 10,
                trainer_required: true,
                eligible: state.sp >= 10,
                unmet: if state.sp >= 10 {
                    vec![]
                } else {
                    vec!["SP required".into()]
                },
            }],
        })
    }
    async fn enqueue_learn_skill(
        &self,
        caller: AccountId,
        character: CharacterId,
        generation: SessionGeneration,
        key: IdempotencyKey,
        skill: SkillRef,
    ) -> Result<LearnSkillReply, AppError> {
        // Give concurrent callers the same chance to observe Unknown before serialized actor
        // settlement. Every enqueue must independently recheck the durable receipt.
        tokio::task::yield_now().await;
        let mut state = self.0.lock();
        state.enqueues = state.enqueues.checked_add(1).unwrap();
        let (send, reply) = tokio::sync::oneshot::channel();
        if state.drop_reply {
            return Ok(reply);
        }
        let fingerprint = learn_skill_fingerprint(character, skill);
        let result = if let Some((stored, result)) = state.receipts.get(&(caller, key)) {
            if *stored == fingerprint {
                Ok(result.clone())
            } else {
                Err(AppError::IdempotencyConflict)
            }
        } else {
            Self::check(&state, caller, character, generation)?;
            if skill.rank != state.rank.checked_add(1).unwrap() || state.sp < 10 {
                Err(AppError::FailedPrecondition("wrong rank or insufficient SP".into()))
            } else {
                state.rank = skill.rank;
                state.sp = state.sp.checked_sub(10).unwrap();
                let result = FrozenLearnResult {
                    character_id: character,
                    skill,
                    sp_spent: 10,
                    sp_remaining: state.sp,
                    tick: Tick(73),
                    hp: 81,
                    max_hp: 101,
                    mp: 42,
                    max_mp: 62,
                    cp: 23,
                    max_cp: 43,
                    effective_stats: LearnedEffectiveStats {
                        p_atk_q: 7_000_001,
                        p_def_q: 8_000_002,
                        m_atk_q: 9_000_003,
                        m_def_q: 10_000_004,
                        attack_speed: 311,
                        cast_speed: 312,
                        speed_q: 11_000_005,
                        accuracy_q: 12_000_006,
                        evasion_q: 13_000_007,
                        crit_permille: 51,
                    },
                    incarnation: 4,
                };
                state
                    .receipts
                    .insert((caller, key), (fingerprint, result.clone()));
                state.writes = state.writes.checked_add(1).unwrap();
                state.events = state.events.checked_add(1).unwrap();
                Ok(result)
            }
        };
        let _ = send.send(result);
        Ok(reply)
    }
}

pub struct Catalogue {
    pub failure: Option<&'static str>,
}
impl SkillCataloguePort for Catalogue {
    fn catalogue(&self) -> Result<SkillCatalogue, AppError> {
        match self.failure {
            Some("unavailable") => {
                return Err(AppError::Unavailable("catalogue unavailable".into()))
            },
            Some(_) => {
                return Err(AppError::Infrastructure(anyhow::anyhow!("private catalogue failure")))
            },
            None => {},
        }
        Ok(SkillCatalogue {
            data_version: "test-registry-version".into(),
            skills: vec![
                SkillCatalogueEntry {
                    skill_id: SkillId(3),
                    key: "l2.skill.3".into(),
                    display_name: "Power Strike".into(),
                    max_level: 9,
                    effect_implemented: true,
                    deferred_reasons: vec![],
                    l2_ref: "Power Strike".into(),
                    // Pinned l2j-dp skills/00000-00099.xml:105-124;
                    // missing MP1/HP: l2j-game model/skills/Skill.java:205-208.
                    mp_consume_start: vec![0; 9],
                    mp_consume_end: vec![9, 9, 10, 11, 12, 13, 16, 16, 17],
                    hp_consume: vec![0; 9],
                    reuse_ms: vec![3000; 9],
                    hit_time_ms: vec![1080; 9],
                },
                SkillCatalogueEntry {
                    skill_id: SkillId(5),
                    key: "l2.skill.5".into(),
                    display_name: "Other".into(),
                    max_level: 9,
                    effect_implemented: true,
                    deferred_reasons: vec![],
                    l2_ref: "Other".into(),
                    mp_consume_start: vec![0; 9],
                    mp_consume_end: vec![0; 9],
                    hp_consume: vec![0; 9],
                    reuse_ms: vec![0; 9],
                    hit_time_ms: vec![0; 9],
                },
                SkillCatalogueEntry {
                    skill_id: SkillId(4),
                    key: "l2.skill.4".into(),
                    display_name: "Deferred".into(),
                    max_level: 1,
                    effect_implemented: false,
                    deferred_reasons: vec!["item_consume".into()],
                    l2_ref: "Deferred".into(),
                    mp_consume_start: vec![0],
                    mp_consume_end: vec![0],
                    hp_consume: vec![0],
                    reuse_ms: vec![0],
                    hit_time_ms: vec![0],
                },
            ],
        })
    }
}
