//! Real-socket Phase 3 use-case/mapping tests with explicitly injected fake runtime.
//! Production composition remains unavailable until Z's actor + checkpoint wiring lands.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use nightfall_api as api;
#[path = "common/phase3.rs"]
mod support;
use api::application::use_cases::{GetSkillCatalog, LearnSkill, ListLearnableSkills};
use api::domain::{AccountId, Character, CharacterName, Race};
use api::infrastructure::memory::InMemoryCharacterRepository;
use api::interface::grpc::{pb, Phase3UseCases};
use common::TestApp;
use std::sync::Arc;
use support::{Catalogue, Runtime};
use tonic::Code;
use uuid::Uuid;

async fn fixture() -> (TestApp, Arc<InMemoryCharacterRepository>, Character, Arc<Runtime>) {
    fixture_with_catalogue(None).await
}

async fn fixture_with_catalogue(
    failure: Option<&'static str>,
) -> (TestApp, Arc<InMemoryCharacterRepository>, Character, Arc<Runtime>) {
    let repo = Arc::new(InMemoryCharacterRepository::default());
    let character = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("Learner").unwrap(),
        Race::Human,
    );
    repo.insert_for_test(character.clone());
    let runtime = Arc::new(Runtime::new(character.account_id, character.id));
    let catalogue = Arc::new(Catalogue { failure });
    let use_cases = Phase3UseCases {
        learn_skill: LearnSkill::new(
            repo.clone(),
            runtime.clone(),
            catalogue.clone(),
            Some(runtime.clone()),
        ),
        list_learnable_skills: ListLearnableSkills::new(repo.clone(), Some(runtime.clone())),
        get_skill_catalog: GetSkillCatalog::new(Some(catalogue)),
    };
    let app = TestApp::spawn_with_phase3(repo.clone(), use_cases).await;
    (app, repo, character, runtime)
}
fn request(character: &Character) -> pb::LearnSkillRequest {
    pb::LearnSkillRequest {
        character_id: character.id.to_string(),
        skill_id: 3,
        skill_level: 1,
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

#[tokio::test]
async fn learning_maps_every_frozen_field_and_replays_after_disconnect_progression() {
    let (mut app, repo, mut character, runtime) = fixture().await;
    let req = request(&character);
    let out = app
        .grpc
        .learn_skill(req.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        out,
        pb::LearnSkillResponse {
            character_id: character.id.to_string(),
            skill_id: 3,
            skill_level: 1,
            sp_spent: 10,
            sp_remaining: 90,
            tick: 73,
            hp: 81,
            max_hp: 101,
            mp: 42,
            max_mp: 62,
            cp: 23,
            max_cp: 43,
            incarnation: 4,
            effective_stats: Some(pb::EffectiveStats {
                p_atk_q: 7_000_001,
                p_def_q: 8_000_002,
                m_atk_q: 9_000_003,
                m_def_q: 10_000_004,
                attack_speed: 311,
                cast_speed: 312,
                speed_q: 11_000_005,
                accuracy_q: 12_000_006,
                evasion_q: 13_000_007,
                crit_permille: 51
            })
        }
    );
    let read = app
        .grpc
        .list_learnable_skills(pb::ListLearnableSkillsRequest {
            character_id: req.character_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.sp, 90);
    assert_eq!(read.skills[0].skill.as_ref().unwrap().skill_level, 2);
    character.level = 60;
    character.class_state.sp = 777;
    repo.insert_for_test(character);
    runtime.0.lock().generation = None;
    assert_eq!(
        app.grpc
            .learn_skill(req.clone())
            .await
            .unwrap()
            .into_inner(),
        out
    );
    let mut other = req;
    other.skill_level = 2;
    assert_eq!(app.grpc.learn_skill(other).await.unwrap_err().code(), Code::FailedPrecondition);
    assert_eq!(runtime.0.lock().writes, 1);
    assert_eq!(runtime.0.lock().events, 1);
}

#[tokio::test]
async fn query_and_catalogue_preserve_flags_metadata_and_named_blockers() {
    let (mut app, _, character, runtime) = fixture().await;
    let query = pb::ListLearnableSkillsRequest {
        character_id: character.id.to_string(),
    };
    let out = app
        .grpc
        .list_learnable_skills(query.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        out,
        pb::ListLearnableSkillsResponse {
            current_class_id: 0,
            sp: 100,
            skills: vec![pb::LearnableSkill {
                learned: false,
                affordable: true,
                trainer_required: true,
                eligible: true,
                unmet: vec![],
                skill: Some(pb::SkillLearnInfo {
                    key: "l2.skill.3".into(),
                    skill_id: 3,
                    l2_ref: "Power Strike".into(),
                    learned_by_npc: true,
                    effect_implemented: true,
                    skill_level: 1,
                    required_level: 1,
                    sp_cost: 10,
                    auto_get: false,
                    required_items: vec![]
                })
            }]
        }
    );
    runtime.0.lock().sp = 0;
    let out = app
        .grpc
        .list_learnable_skills(query)
        .await
        .unwrap()
        .into_inner();
    assert!(!out.skills[0].eligible);
    assert!(!out.skills[0].affordable);
    assert_eq!(out.skills[0].unmet, ["SP required"]);
    let catalog = app
        .grpc
        .get_skill_catalog(pb::GetSkillCatalogRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(catalog.data_version, "test-registry-version");
    assert_eq!(
        catalog.skills[0],
        pb::SkillCatalogEntry {
            skill_id: 3,
            key: "l2.skill.3".into(),
            display_name: "Power Strike".into(),
            max_level: 9,
            effect_implemented: true,
            deferred_reasons: vec![],
            l2_ref: "Power Strike".into()
        }
    );
    let deferred = catalog.skills.iter().find(|s| s.skill_id == 4).unwrap();
    assert!(!deferred.effect_implemented);
    assert_eq!(deferred.deferred_reasons, ["item_consume"]);
    assert_eq!(runtime.0.lock().writes, 0);
}

#[tokio::test]
async fn authenticated_errors_cover_every_documented_learning_code_without_consuming_key() {
    let (mut app, _, character, runtime) = fixture().await;
    let req = request(&character);
    let mut anon = app.anon_grpc();
    assert_eq!(anon.learn_skill(req.clone()).await.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(
        anon.list_learnable_skills(pb::ListLearnableSkillsRequest {
            character_id: req.character_id.clone()
        })
        .await
        .unwrap_err()
        .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        anon.get_skill_catalog(pb::GetSkillCatalogRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.game_as(common::other_account())
            .learn_skill(req.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    for case in 0..7 {
        let mut invalid = req.clone();
        match case {
            0 => invalid.character_id = "bad".into(),
            1 => invalid.idempotency_key.clear(),
            2 => invalid.idempotency_key = "bad".into(),
            3 => invalid.skill_id = 0,
            4 => invalid.skill_level = 0,
            5 => invalid.skill_id = 999,
            _ => invalid.skill_level = 10,
        }
        assert_eq!(app.grpc.learn_skill(invalid).await.unwrap_err().code(), Code::InvalidArgument);
    }
    let mut missing = req.clone();
    missing.character_id = Uuid::nil().to_string();
    assert_eq!(app.grpc.learn_skill(missing).await.unwrap_err().code(), Code::NotFound);
    let mut deferred = req.clone();
    deferred.skill_id = 4;
    assert_eq!(
        app.grpc.learn_skill(deferred).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    for (reason, code) in [
        ("dead", Code::FailedPrecondition),
        ("engaged", Code::FailedPrecondition),
        ("casting", Code::FailedPrecondition),
        ("trainer required", Code::FailedPrecondition),
        ("lineage", Code::FailedPrecondition),
        ("rank", Code::FailedPrecondition),
        ("required level", Code::FailedPrecondition),
        ("SP", Code::FailedPrecondition),
        ("learning item prerequisite", Code::FailedPrecondition),
        ("unavailable", Code::Unavailable),
        ("infrastructure", Code::Internal),
        ("resource_exhausted", Code::ResourceExhausted),
    ] {
        runtime.0.lock().failure = Some(reason);
        let error = app.grpc.learn_skill(req.clone()).await.unwrap_err();
        assert_eq!(error.code(), code);
        if code == Code::Internal {
            assert_eq!(error.message(), "internal error");
        }
        assert!(runtime.0.lock().receipts.is_empty());
        assert_eq!(runtime.0.lock().sp, 100);
    }
    runtime.0.lock().failure = None;
    runtime.0.lock().stale_after_capture = true;
    assert_eq!(
        app.grpc.learn_skill(req.clone()).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    runtime.0.lock().stale_after_capture = false;
    runtime.0.lock().generation = None;
    assert_eq!(
        app.grpc.learn_skill(req.clone()).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    runtime.0.lock().generation = Some(api::domain::SessionGeneration::FIRST);
    assert!(app.grpc.learn_skill(req).await.is_ok());
    assert_eq!(runtime.0.lock().writes, 1);
}

#[tokio::test]
async fn query_error_codes_are_generation_fenced_and_do_not_write() {
    let (mut app, _, character, runtime) = fixture().await;
    let query = pb::ListLearnableSkillsRequest {
        character_id: character.id.to_string(),
    };
    assert_eq!(
        app.game_as(common::other_account())
            .list_learnable_skills(query.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    for (id, code) in [
        ("bad".to_owned(), Code::InvalidArgument),
        (Uuid::nil().to_string(), Code::NotFound),
    ] {
        assert_eq!(
            app.grpc
                .list_learnable_skills(pb::ListLearnableSkillsRequest { character_id: id })
                .await
                .unwrap_err()
                .code(),
            code
        );
    }
    for (reason, code) in [
        ("unavailable", Code::Unavailable),
        ("infrastructure", Code::Internal),
    ] {
        runtime.0.lock().failure = Some(reason);
        let error = app
            .grpc
            .list_learnable_skills(query.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        if code == Code::Internal {
            assert_eq!(error.message(), "internal error");
        }
    }
    runtime.0.lock().failure = None;
    runtime.0.lock().generation = None;
    assert_eq!(
        app.grpc
            .list_learnable_skills(query)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(runtime.0.lock().writes, 0);
}

#[tokio::test]
async fn catalogue_failures_return_documented_codes_and_hide_infrastructure_details() {
    for (reason, code) in [
        ("unavailable", Code::Unavailable),
        ("infrastructure", Code::Internal),
    ] {
        let (mut app, _, _, runtime) = fixture_with_catalogue(Some(reason)).await;
        let error = app
            .grpc
            .get_skill_catalog(pb::GetSkillCatalogRequest {})
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        if code == Code::Internal {
            assert_eq!(error.message(), "internal error");
        }
        assert_eq!(runtime.0.lock().writes, 0);
    }
}

#[tokio::test]
async fn concurrent_rpc_duplicates_return_one_frozen_reply_and_different_bodies_conflict() {
    let (app, _, character, runtime) = fixture().await;
    let req = request(&character);
    let mut first = app.grpc.clone();
    let mut second = app.grpc.clone();
    let (a, b) = tokio::join!(first.learn_skill(req.clone()), second.learn_skill(req.clone()));
    assert_eq!(a.unwrap().into_inner(), b.unwrap().into_inner());
    assert_eq!(runtime.0.lock().writes, 1);
    let mut other = req;
    other.skill_id = 5;
    assert_eq!(first.learn_skill(other).await.unwrap_err().code(), Code::FailedPrecondition);
    assert_eq!(runtime.0.lock().events, 1);
}
