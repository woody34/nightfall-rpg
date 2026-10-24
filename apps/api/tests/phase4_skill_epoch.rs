//! V6 promoted manual ranks persist under the exact compiled epoch; legacy policy stays strict.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::character_progression::{
    ClassState, FrozenTransferResult, SuccessfulTransferReceipt,
};
use nightfall_api::domain::class::{ClassId, ClassRegistry, RuntimeSkillStatus};
use nightfall_api::domain::subclass::LearnedSkill;
use nightfall_api::domain::zone::{skill::SkillId, Phase3Checkpoint};
use nightfall_api::domain::{AccountId, Character, CharacterName, Position, Race};
use nightfall_api::infrastructure::{
    class_data::{load_classes, ClassSource},
    item_data::{embedded_phase4_bundle, Phase4RuleBundle},
    memory::InMemoryCharacterRepository,
    postgres::PgCharacterRepository,
    rules_data::{load_rules, RulesSource},
};
use uuid::Uuid;

fn character(classes: &ClassRegistry) -> Character {
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("PromotedProphet").unwrap(),
        Race::Human,
    );
    c.class_state = ClassState::new(ClassId(10));
    c.stats = classes.get(ClassId(10)).unwrap().base_stats;
    c
}
fn checkpoint(c: &Character) -> CharacterCheckpoint {
    let rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let mut ledger = c.class_state.clone();
    ledger.current_class_id = ClassId(17);
    for (target, level) in [(15, 20), (17, 40)] {
        ledger
            .record_success(SuccessfulTransferReceipt {
                key: Uuid::from_u128(u128::from(target)),
                target_class_id: ClassId(target),
                result: FrozenTransferResult {
                    character_id: c.id,
                    identity: c.identity(),
                    name: c.name.clone(),
                    current_class_id: ClassId(target),
                    level,
                    xp: rules.xp_to_level(level).unwrap(),
                    sp: 0,
                    stats: c.stats,
                    position_millitiles: [126_000, 126_000],
                    hp: 1,
                    mp: 1,
                    cp: 0,
                    max_hp: 1,
                    max_mp: 1,
                    max_cp: 0,
                    token_tier_1_count: 0,
                    token_tier_2_count: 0,
                    granted_skill_keys: Vec::new(),
                },
            })
            .unwrap();
    }
    ledger.merge_learned_skills([LearnedSkill {
        key: "l2.skill.1388".into(),
        level: 1,
    }]);
    CharacterCheckpoint {
        character_id: c.id,
        revision_seen: 0,
        level: 60,
        xp: rules.xp_to_level(60).unwrap(),
        hp: 1,
        mp: 1,
        alive: true,
        position: Position::default(),
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
        phase3: Some(Phase3Checkpoint::new()),
        phase4: None,
    }
}
async fn contract(
    v6: &dyn CharacterRepository,
    legacy: &dyn CharacterRepository,
    classes: &ClassRegistry,
    bundle: &Phase4RuleBundle,
) {
    let c = character(classes);
    let cp = checkpoint(&c);
    let ledger = cp.class_state.as_ref().unwrap();
    assert_eq!(bundle.skills.runtime_revision(), 3);
    assert_eq!(
        bundle
            .skills
            .skills()
            .get(SkillId(1388))
            .unwrap()
            .runtime_status,
        RuntimeSkillStatus::Executable
    );
    assert!(ledger
        .validate_for_phase3(classes, &c.identity(), c.id)
        .is_err());
    ledger
        .validate_for_skill_rules(classes, &c.identity(), c.id, &bundle.skills)
        .unwrap();
    for repo in [v6, legacy] {
        repo.create_idempotent(&IdempotencyKey::new(), "promoted-prophet", &c)
            .await
            .unwrap();
    }
    assert!(matches!(legacy.checkpoint(&cp, &[]).await, Err(CheckpointError::Constraint(_))));
    assert_eq!(
        legacy
            .load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    assert_eq!(v6.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Applied(1));
    assert_eq!(v6.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Replayed(1));
    let committed = v6.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(committed.class_state, *ledger);
    assert_eq!(committed.phase3, cp.phase3);
    assert_eq!(v6.get(c.id).await.unwrap().unwrap().class_state, *ledger);
    assert_eq!(
        v6.list_by_account(c.account_id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .class_state,
        *ledger
    );
    for skill in [
        LearnedSkill {
            key: "l2.skill.1388".into(),
            level: u32::MAX,
        },
        LearnedSkill {
            key: "l2.skill.248".into(),
            level: 1,
        },
    ] {
        let mut invalid = cp.clone();
        invalid.revision_seen = 1;
        invalid.idempotency.1 = IdempotencyKey::new();
        invalid.class_state.as_mut().unwrap().learned_skills = vec![skill];
        assert!(matches!(
            v6.checkpoint(&invalid, &[]).await,
            Err(CheckpointError::Constraint(_))
        ));
        assert_eq!(v6.load_for_admission(c.id).await.unwrap().unwrap(), committed);
    }
}
#[tokio::test]
async fn memory_validates_promoted_manual_ranks_against_v6_without_widening_legacy() {
    let bundle = embedded_phase4_bundle().unwrap();
    let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
    let v6 = InMemoryCharacterRepository::default()
        .with_classes(classes.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let legacy = InMemoryCharacterRepository::default().with_classes(classes.clone());
    contract(&v6, &legacy, &classes, &bundle).await;
}
#[tokio::test]
async fn postgres_validates_promoted_manual_ranks_against_v6_without_widening_legacy() {
    let Some(v6_pool) = common::pg::migrated_pool().await else {
        return;
    };
    let Some(legacy_pool) = common::pg::migrated_pool().await else {
        return;
    };
    let bundle = embedded_phase4_bundle().unwrap();
    let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
    let v6 = PgCharacterRepository::new(v6_pool)
        .with_classes(classes.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let legacy = PgCharacterRepository::new(legacy_pool).with_classes(classes.clone());
    contract(&v6, &legacy, &classes, &bundle).await;
}
