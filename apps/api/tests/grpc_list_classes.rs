#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;
use common::TestApp;
use nightfall_api::interface::grpc::pb;
use tonic::Code;

#[tokio::test]
async fn catalogue_contains_all_races_professions_limits_and_appearance_hooks() {
    let mut app = TestApp::spawn().await;
    let response = app
        .grpc
        .list_classes(pb::ListClassesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.data_version.len(), 64);
    assert_eq!((response.playable_level_cap, response.max_transfer_tier), (85, 2));
    assert_eq!(
        response.class_master,
        Some(pb::ClassMasterInfo {
            name: "Class Master".into(),
            position: Some(pb::Position { x: 126.0, y: 128.0 }),
            interaction_radius: 3.0
        })
    );
    use prost::Message;
    let encoded_len = response.encoded_len();
    println!("ListClassesResponse encoded_len={encoded_len} bytes; tonic limit=4194304 bytes");
    assert!(
        encoded_len < 4 * 1024 * 1024,
        "catalogue exceeds tonic default message limit: {encoded_len}"
    );
    assert_eq!(
        response
            .classes
            .iter()
            .filter(|c| c.skill_tree_populated)
            .count(),
        39
    );
    assert_eq!(
        response
            .classes
            .iter()
            .map(|c| c.skill_tree.len())
            .sum::<usize>(),
        6927
    );
    assert_eq!(
        response
            .classes
            .iter()
            .map(|c| c.proficiencies.len())
            .sum::<usize>(),
        3521
    );
    assert!(response
        .classes
        .iter()
        .flat_map(|c| &c.skill_tree)
        .all(|s| s.skill_id > 0 && !s.l2_ref.is_empty() && !s.effect_implemented));
    assert!(response
        .classes
        .iter()
        .flat_map(|c| &c.proficiencies)
        .all(|s| s.skill_id > 0
            && s.skill_level > 0
            && !s.l2_ref.is_empty()
            && !s.effect_implemented));
    assert_eq!(response.races.len(), 5);
    assert_eq!(response.classes.len(), 89);
    assert_eq!(
        (0..=3)
            .map(|tier| response.classes.iter().filter(|c| c.tier == tier).count())
            .collect::<Vec<_>>(),
        vec![9, 18, 31, 31]
    );
    assert!(response
        .classes
        .windows(2)
        .all(|pair| pair[0].class_id < pair[1].class_id));
    for race in &response.races {
        assert_eq!((race.hair_style_count, race.hair_color_count, race.face_count), (1, 1, 1));
        assert!(!race.display_name.is_empty());
        assert!(race.walk_speed > 0 && race.run_speed > 0);
        assert_eq!(race.mystic_path, race.race != pb::Race::Dwarf as i32);
        assert_eq!(race.base_class_ids.len(), if race.mystic_path { 2 } else { 1 });
        assert_eq!(
            race.passive_skill_keys,
            race.passives
                .iter()
                .map(|p| p.key.clone())
                .collect::<Vec<_>>()
        );
        assert!(race
            .passives
            .iter()
            .all(|p| !p.display_name.is_empty() && !p.description.is_empty()));
        for passive in &race.passives {
            assert_eq!(
                passive.implemented,
                matches!(
                    passive.key.as_str(),
                    "racial.adaptable" | "racial.forest_step" | "racial.shadow_precision"
                ),
                "{}",
                passive.key
            );
        }
        for id in &race.base_class_ids {
            let class = response.classes.iter().find(|c| c.class_id == *id).unwrap();
            assert_eq!(
                (class.race, class.tier, class.parent_class_id, class.min_level),
                (race.race, 0, 65535, 1)
            );
        }
    }
    for class in &response.classes {
        assert!(!class.key.is_empty() && !class.display_name.is_empty());
        assert!(class.walk_speed > 0 && class.run_speed > 0 && class.swim_speed > 0);
        assert!(
            class.archetype == pb::Archetype::Fighter as i32
                || class.archetype == pb::Archetype::Mystic as i32
        );
        let stats = class.base_stats.as_ref().unwrap();
        assert_eq!(stats.str + stats.dex + stats.con + stats.int + stats.wit + stats.men, 170);
        if class.tier > 0 {
            let parent = response
                .classes
                .iter()
                .find(|p| p.class_id == class.parent_class_id)
                .unwrap();
            assert_eq!(parent.tier + 1, class.tier);
            assert_eq!(parent.race, class.race);
        }
    }
    let fighter = response.classes.iter().find(|c| c.class_id == 0).unwrap();
    let mystic = response.classes.iter().find(|c| c.class_id == 10).unwrap();
    assert_eq!((fighter.walk_speed, fighter.run_speed), (80, 115));
    assert_eq!((mystic.walk_speed, mystic.run_speed), (78, 120));
    assert_eq!(
        app.grpc
            .list_classes(pb::ListClassesRequest {})
            .await
            .unwrap()
            .into_inner(),
        response
    );
    assert!(app.characters.is_empty());
    assert!(app.characters.staged_events().is_empty());
}

#[tokio::test]
async fn catalogue_requires_authentication() {
    let app = TestApp::spawn().await;
    assert_eq!(
        app.anon_grpc()
            .list_classes(pb::ListClassesRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert!(app.accounts.is_empty());
}
