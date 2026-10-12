//! Complete catalogue transport/error projection using immutable embedded metadata fixtures.
//! Production registry binding to the actual V6 skills is separately fail closed.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use common::TestApp;
use nightfall_api::application::{
    ports::ItemCataloguePort,
    use_cases::{GetItemCatalog, RegistryItemCatalogue},
    AppError,
};
use nightfall_api::domain::zone::Phase4Rules;
use nightfall_api::interface::grpc::pb;
use std::sync::{Arc, OnceLock};
use tonic::Code;

fn fixture_rules() -> Arc<Phase4Rules> {
    static RULES: OnceLock<Arc<Phase4Rules>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            Arc::new(
                nightfall_api::infrastructure::item_data::load_embedded_phase4_rules([7; 32])
                    .unwrap(),
            )
        })
        .clone()
}
struct Fixture(Arc<Phase4Rules>);
impl ItemCataloguePort for Fixture {
    fn catalogue(&self) -> Result<Arc<Phase4Rules>, AppError> {
        Ok(self.0.clone())
    }
}
struct Failure(bool);
impl ItemCataloguePort for Failure {
    fn catalogue(&self) -> Result<Arc<Phase4Rules>, AppError> {
        if self.0 {
            Err(AppError::Infrastructure(anyhow::anyhow!("sentinel internal catalogue failure")))
        } else {
            Err(AppError::Unavailable("catalogue rollout unavailable".into()))
        }
    }
}

#[test]
fn use_case_read_is_immutable_and_each_reachable_port_error_is_propagated() {
    let rules = fixture_rules();
    let read = GetItemCatalog::new(Some(Arc::new(Fixture(rules.clone()))));
    assert!(Arc::ptr_eq(&read.execute().unwrap(), &rules));
    assert!(Arc::ptr_eq(&read.execute().unwrap(), &rules));
    assert!(matches!(GetItemCatalog::new(None).execute(), Err(AppError::Unavailable(_))));
    assert!(matches!(
        GetItemCatalog::new(Some(Arc::new(Failure(false)))).execute(),
        Err(AppError::Unavailable(_))
    ));
    assert!(matches!(
        GetItemCatalog::new(Some(Arc::new(Failure(true)))).execute(),
        Err(AppError::Infrastructure(_))
    ));
    let old_skills = nightfall_api::infrastructure::skill_data::load_phase3_rules(
        &nightfall_api::infrastructure::skill_data::Phase3DataSource::embedded(),
        &[],
    )
    .unwrap();
    assert!(matches!(
        RegistryItemCatalogue::new(rules, &old_skills),
        Err(AppError::Unavailable(_))
    ));
}

#[tokio::test]
async fn immutable_full_catalogue_is_authenticated_complete_and_character_independent() {
    let rules = fixture_rules();
    let configured = rules.clone();
    let app =
        TestApp::spawn_with(move |deps| deps.item_catalogue = Some(Arc::new(Fixture(configured))))
            .await;
    assert_eq!(
        app.anon_grpc()
            .get_item_catalog(pb::GetItemCatalogRequest::default())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    let response = app
        .grpc
        .clone()
        .get_item_catalog(pb::GetItemCatalogRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.data_version, 2);
    assert_eq!(response.rules_hash, rules.hash());
    assert_eq!(response.items.len(), rules.parts().items.len());
    assert_eq!(response.capabilities.len(), response.items.len());
    for (item, source) in response.items.iter().zip(rules.parts().items.values()) {
        assert_eq!(item.template_id, source.template_id);
        assert_eq!(item.name, source.name);
        assert_eq!(item.bodypart_mask, source.bodypart_mask);
        assert_eq!(item.weight, source.weight);
        assert_eq!(item.stackable, source.stackable);
        assert_eq!(item.icon_key, source.icon);
        assert_eq!(item.usable, item.blockers.is_empty());
        assert_ne!(item.kind, pb::ItemKind::Unspecified as i32);
        assert_ne!(item.grade, pb::ItemGrade::Unspecified as i32);
        let expected_kind = match source.kind.as_str() {
            "Weapon" => pb::ItemKind::Weapon,
            "Armor" => pb::ItemKind::Armor,
            "EtcItem" => pb::ItemKind::Etc,
            _ => pb::ItemKind::Unspecified,
        };
        assert_eq!(item.kind, i32::from(expected_kind));
        assert_eq!(item.stats.len(), source.functions.len());
        for (actual, function) in item.stats.iter().zip(&source.functions) {
            let (numerator, denominator) = function.value.value().parts();
            assert_eq!((&actual.stat, &actual.operation), (&function.stat, &function.operation));
            assert_eq!(actual.numerator, i64::try_from(numerator).unwrap());
            assert_eq!(actual.denominator, i64::try_from(denominator).unwrap());
            assert_eq!(actual.order, u32::try_from(function.order).unwrap());
        }
        assert_eq!(item.skills.len(), source.skills.len() + source.enchant4_skills.len());
        for (actual, skill) in item
            .skills
            .iter()
            .zip(source.skills.iter().chain(&source.enchant4_skills))
        {
            assert_eq!((actual.id, actual.rank), (skill.id, skill.level));
        }
    }
    for (cap, item) in response.capabilities.iter().zip(&response.items) {
        assert_eq!(cap.template_id, item.template_id);
        assert_eq!(cap.blockers, item.blockers);
    }
    let admitted = rules.parts().sa_recipes.values().filter(|recipe| {
        rules
            .parts()
            .recipe_capabilities
            .get(&recipe.recipe_id)
            .unwrap()
            .admitted
    });
    assert_eq!(response.recipes.len(), admitted.clone().count());
    for (actual, source) in response.recipes.iter().zip(admitted) {
        assert_eq!(actual.recipe_id, source.recipe_id);
        assert_eq!(actual.base_template, source.base_template);
        assert_eq!(actual.product_template, source.products.first().unwrap().template_id);
        assert_eq!(actual.maintain_enchantment, source.maintain_enchantment);
        assert_eq!(actual.ingredients.len(), source.ingredients.len());
        for (actual, ingredient) in actual.ingredients.iter().zip(&source.ingredients) {
            assert_eq!((actual.template, actual.count), (ingredient.template_id, ingredient.count));
        }
    }
    let repeated = app
        .game_as(uuid::Uuid::from_u128(99))
        .get_item_catalog(pb::GetItemCatalogRequest {
            known_rules_hash: Some(response.rules_hash.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(repeated, response);
    assert!(app.characters.staged_events().is_empty());
    assert!(app.bus.published().is_empty());
}

#[tokio::test]
async fn absent_unsupported_and_corrupt_catalogues_are_fail_closed_with_sanitized_status() {
    for (port, code) in [
        (None, Code::Unavailable),
        (Some(Arc::new(Failure(false)) as Arc<dyn ItemCataloguePort>), Code::Unavailable),
        (Some(Arc::new(Failure(true)) as Arc<dyn ItemCataloguePort>), Code::Internal),
    ] {
        let app = TestApp::spawn_with(move |deps| deps.item_catalogue = port).await;
        let error = app
            .grpc
            .clone()
            .get_item_catalog(pb::GetItemCatalogRequest::default())
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        if code == Code::Internal {
            assert_eq!(error.message(), "internal error");
        }
        assert!(!error.message().contains("sentinel"));
        assert!(app.characters.staged_events().is_empty());
        assert!(app.bus.published().is_empty());
    }
}
