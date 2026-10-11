//! Exporter-level bounded label/cardinality contract and authenticated RPC accounting.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use nightfall_api::infrastructure::telemetry::{
    Metrics, SkillCastOutcome, SkillEffectChange, SkillEffectKind, SkillLearnOutcome, SkillOutcome,
    SkillResolutionKind,
};
use nightfall_api::interface::grpc::pb;
use std::time::Duration;

fn samples<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    text.lines()
        .filter(|line| {
            line.starts_with(&format!("{name}{{")) || line.starts_with(&format!("{name} "))
        })
        .collect()
}
#[test]
fn bounded_series_initialize_at_zero_and_only_typed_recorders_increment() {
    let metrics = Metrics::detached();
    let initial = metrics.render().unwrap();
    for (name, count) in [
        ("nightfall_skill_casts_total", 3),
        ("nightfall_skill_resolutions_total", 25),
        ("nightfall_skill_effects_total", 48),
        ("nightfall_skill_learn_requests_total", 4),
        ("nightfall_skills_learned_total", 1),
    ] {
        let series = samples(&initial, name);
        assert_eq!(series.len(), count, "{name}");
        assert!(series.iter().all(|line| line.ends_with(" 0")));
    }
    assert!(samples(&initial, "nightfall_skill_learn_seconds_count").is_empty());
    metrics.record_skill_cast(SkillCastOutcome::Started);
    metrics.record_skill_cast(SkillCastOutcome::Completed);
    metrics.record_skill_cast(SkillCastOutcome::Cancelled);
    metrics.record_skill_resolution(SkillResolutionKind::Magical, SkillOutcome::Half);
    metrics.record_skill_effect(SkillEffectKind::TickHp, SkillEffectChange::Applied);
    metrics.record_skill_effect(SkillEffectKind::TickHp, SkillEffectChange::Removed);
    metrics.record_skill_learn_request(Duration::from_millis(2), SkillLearnOutcome::Success);
    metrics.record_skill_learn_request(Duration::from_millis(3), SkillLearnOutcome::Success); // receipt replay is another RPC attempt
    metrics.record_skill_learn_committed(); // only one applied checkpoint, excludes replay
    let text = metrics.render().unwrap();
    for (name, labels, value) in [
        ("nightfall_skill_casts_total", vec!["outcome=\"started\""], 1),
        (
            "nightfall_skill_resolutions_total",
            vec!["kind=\"magical\"", "outcome=\"half\""],
            1,
        ),
        (
            "nightfall_skill_effects_total",
            vec!["kind=\"tick_hp\"", "change=\"applied\""],
            1,
        ),
        ("nightfall_skill_learn_requests_total", vec!["outcome=\"success\""], 2),
        ("nightfall_skill_learn_seconds_count", vec!["outcome=\"success\""], 2),
        ("nightfall_skills_learned_total", vec![], 1),
    ] {
        let row = samples(&text, name)
            .into_iter()
            .find(|line| labels.iter().all(|label| line.contains(label)))
            .unwrap();
        assert!(row.ends_with(&format!(" {value}")), "{row}");
    }
    for row in text
        .lines()
        .filter(|line| line.starts_with("nightfall_skill"))
    {
        for forbidden in [
            "skill_id=",
            "entity_id=",
            "character_id=",
            "account_id=",
            "session_id=",
            "cast_id=",
            "effect_id=",
            "idempotency_key=",
        ] {
            assert!(!row.contains(forbidden), "{row}");
        }
    }
}
#[tokio::test]
async fn rollout_learning_rpc_is_unavailable_attempt_and_never_committed_purchase() {
    let mut app = common::TestApp::spawn().await;
    let req = pb::LearnSkillRequest {
        character_id: "not-a-uuid".into(),
        skill_id: 3,
        skill_level: 1,
        idempotency_key: "secret-request-key".into(),
    };
    assert_eq!(
        app.anon_grpc()
            .learn_skill(req.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    for _ in 0..2 {
        assert_eq!(
            app.grpc.learn_skill(req.clone()).await.unwrap_err().code(),
            tonic::Code::Unavailable
        );
    }
    let text = app.metrics.render().unwrap();
    let row = samples(&text, "nightfall_skill_learn_requests_total")
        .into_iter()
        .find(|row| row.contains("outcome=\"unavailable\""))
        .unwrap();
    assert!(row.ends_with(" 2"), "{row}");
    assert!(samples(&text, "nightfall_skills_learned_total")
        .iter()
        .all(|row| row.ends_with(" 0")));
    assert!(!text.contains("secret-request-key"));
    assert!(!text.contains("not-a-uuid"));
    assert!(app.characters.staged_events().is_empty());
    assert!(app.bus.published().is_empty());
}
