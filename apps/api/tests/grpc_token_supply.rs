//! Approved once-ever supply through real gRPC/WebSockets, Postgres and default-limit NATS.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;

use common::token::{balances, character, grants, spawned, Harness};
use common::ws;
use nightfall_api::application::replay_log::WatermarkReason;
use nightfall_api::application::{CharacterCheckpoint, CharacterRepository, IdempotencyKey};
use nightfall_api::domain::character_progression::{
    FrozenTransferResult, SuccessfulTransferReceipt,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::Character;
use nightfall_api::interface::grpc::pb;
use uuid::Uuid;

async fn save_vitals(h: &Harness, c: &Character, revision: u64) {
    h.repo
        .checkpoint(
            &CharacterCheckpoint {
                character_id: c.id,
                revision_seen: revision,
                level: c.level,
                xp: c.xp,
                hp: 77,
                mp: 31,
                alive: true,
                position: c.position,
                idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
                class_state: Some(c.class_state.clone()),
            },
            &[],
        )
        .await
        .unwrap();
}

fn change(c: &Character, key: Uuid, target: u32) -> pb::ChangeClassRequest {
    pb::ChangeClassRequest {
        character_id: c.id.to_string(),
        target_class_id: target,
        idempotency_key: key.to_string(),
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One service/schema covers the independent admission policy matrix.
async fn sparse_existing_admission_marks_positive_balances_grants_only_unclaimed_eligible_tiers_and_preserves_vitals(
) {
    let h = Harness::new().await;
    for (name, level, initial, mask, expected, claimed, expected_grants) in [
        ("Below", 19, [0, 0], 0, [0, 0], 0, vec![]),
        ("Twenty", 20, [0, 0], 0, [1, 0], 1, vec![1]),
        ("Thirtynine", 39, [0, 0], 0, [1, 0], 1, vec![1]),
        ("Forty", 40, [0, 0], 0, [1, 1], 3, vec![1, 2]),
        ("Cap", 85, [0, 0], 0, [1, 1], 3, vec![1, 2]),
        ("Positivefirst", 19, [3, 0], 0, [3, 0], 1, vec![]),
        ("Positivesecond", 19, [0, 2], 0, [0, 2], 2, vec![]),
        ("Positiveboth", 40, [4, 5], 0, [4, 5], 3, vec![]),
        ("Claimedfirst", 20, [0, 0], 1, [0, 0], 1, vec![]),
        ("Claimedboth", 40, [0, 0], 3, [0, 0], 3, vec![]),
        ("Onlysecond", 40, [0, 0], 1, [0, 1], 3, vec![2]),
        ("Onlyfirst", 40, [0, 0], 2, [1, 0], 3, vec![1]),
        ("Deleveled", 19, [0, 0], 3, [0, 0], 3, vec![]),
    ] {
        let mut c = character(name, level, initial, mask);
        c.class_state.sp = 555;
        c.class_state.cp = 11;
        let p = h.seed(&c).await;
        save_vitals(&h, &c, 0).await;
        let before = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
        let mut socket = ws::join(&h.app, &p).await;
        socket.until(|m| spawned(m, &p)).await;
        // The first visible frame is the barrier: read committed DB state immediately.
        let after = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!(
            balances(&h.repo.get(c.id).await.unwrap().unwrap()),
            (expected, claimed),
            "{name}"
        );
        assert_eq!(
            (after.level, after.xp, after.hp, after.mp, after.alive, after.position),
            (before.level, before.xp, before.hp, before.mp, before.alive, before.position),
            "{name}"
        );
        assert_eq!((after.class_state.sp, after.class_state.cp), (555, 11));
        assert_eq!(after.identity, before.identity);
        assert_eq!(after.class_state.learned_skills, before.class_state.learned_skills);
        assert_eq!(
            after.class_state.successful_transfer_receipts,
            before.class_state.successful_transfer_receipts
        );
        if initial != expected || mask != claimed {
            assert_eq!(
                after.revision,
                before.revision.checked_add(1).unwrap(),
                "critical reconciliation {name}"
            );
        }
        let rows = grants(&h.pool, &c).await;
        assert_eq!(
            rows.iter()
                .map(|v| v["tier"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            expected_grants,
            "{name}"
        );
        for row in &rows {
            assert_eq!(row["source"], "Admission");
        }
        let options = h
            .app
            .game_as(p.account)
            .transfer_options(pb::TransferOptionsRequest {
                character_id: c.id.to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!([options.token_tier_1_count, options.token_tier_2_count], expected);
        // Replacement admits a higher generation while the first socket is still alive.
        // It must use the already committed ledger and stage no additional grant.
        let mut replacement = ws::join(&h.app, &p).await;
        replacement.until(|m| spawned(m, &p)).await;
        assert_eq!(socket.closed(ws::WAIT).await, Some(4409));
        assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), (expected, claimed));
        assert_eq!(grants(&h.pool, &c).await, rows);
        replacement.close().await;
    }
    h.running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

#[tokio::test]
async fn backfill_then_two_consumptions_retry_and_reconnect_preserve_exact_frozen_results_without_spares(
) {
    let h = Harness::new().await;
    let c = character("Backfill", 40, [0, 0], 0);
    let p = h.seed(&c).await;
    let mut socket = ws::join(&h.app, &p).await;
    socket.until(|m| spawned(m, &p)).await;
    assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), ([1, 1], 3));
    let mut client = h.app.game_as(p.account);
    let first_request = change(&c, Uuid::now_v7(), 1);
    let mut concurrent = client.clone();
    let (first, retry) = tokio::join!(
        client.change_class(first_request.clone()),
        concurrent.change_class(first_request.clone())
    );
    let first = first.unwrap().into_inner();
    assert_eq!(retry.unwrap().into_inner(), first);
    assert_eq!(
        first.granted_skill_keys,
        [
            "l2.skill.1320",
            "l2.skill.1322",
            "l2.skill.194",
            "l2.skill.239"
        ]
    );
    assert_eq!((first.token_tier_1_count, first.token_tier_2_count), (0, 1));
    let first_receipt = h
        .repo
        .get(c.id)
        .await
        .unwrap()
        .unwrap()
        .class_state
        .successful_transfer_receipts[0]
        .clone();
    let second_request = change(&c, Uuid::now_v7(), 2);
    let second = client
        .change_class(second_request.clone())
        .await
        .unwrap()
        .into_inner();
    assert!(second.granted_skill_keys.is_empty());
    assert_eq!((second.token_tier_1_count, second.token_tier_2_count), (0, 0));
    let consumed = h.repo.get(c.id).await.unwrap().unwrap();
    assert_eq!(balances(&consumed), ([0, 0], 3));
    assert_eq!(consumed.class_state.successful_transfer_receipts[0], first_receipt);
    let mut replacement = ws::join(&h.app, &p).await;
    replacement.until(|m| spawned(m, &p)).await;
    assert_eq!(socket.closed(ws::WAIT).await, Some(4409));
    assert_eq!(h.repo.get(c.id).await.unwrap().unwrap().class_state, consumed.class_state);
    assert_eq!(
        client
            .change_class(first_request)
            .await
            .unwrap()
            .into_inner(),
        first
    );
    assert_eq!(
        client
            .change_class(second_request)
            .await
            .unwrap()
            .into_inner(),
        second
    );
    assert_eq!(h.repo.get(c.id).await.unwrap().unwrap().class_state, consumed.class_state);
    assert_eq!(grants(&h.pool, &c).await.len(), 2);
    replacement.close().await;
    h.running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

fn receipt(c: &Character, target: u32, key: Uuid) -> SuccessfulTransferReceipt {
    SuccessfulTransferReceipt {
        key,
        target_class_id: ClassId(target),
        result: FrozenTransferResult {
            character_id: c.id,
            identity: c.identity(),
            name: c.name.clone(),
            current_class_id: ClassId(target),
            level: 40,
            xp: 15_422_929,
            sp: 555,
            stats: c.stats,
            position_millitiles: [126_000, 126_000],
            hp: 77,
            mp: 31,
            cp: 11,
            max_hp: 100,
            max_mp: 100,
            max_cp: 100,
            token_tier_1_count: 0,
            token_tier_2_count: 0,
            granted_skill_keys: vec![],
        },
    }
}

#[tokio::test]
async fn validated_completed_receipts_mark_consumed_tiers_even_after_delevel_and_remain_byte_exact()
{
    let h = Harness::new().await;
    for (name, level, tier, expected, mask, grant_count) in [
        ("Completedfirst", 40, 1, [0, 1], 3, 1),
        ("Completedbelow", 19, 1, [0, 0], 1, 0),
        ("Completedboth", 19, 2, [0, 0], 3, 0),
    ] {
        let mut c = character(name, level, [0, 0], 0);
        let p = h.seed(&c).await;
        c.class_state.sp = 555;
        c.class_state.cp = 11;
        for target in 1..=tier {
            c.class_state.current_class_id = ClassId(target);
            c.class_state
                .record_success(receipt(&c, target, Uuid::now_v7()))
                .unwrap();
        }
        save_vitals(&h, &c, 0).await;
        let before = h.repo.get(c.id).await.unwrap().unwrap();
        let frozen_bytes =
            serde_json::to_vec(&before.class_state.successful_transfer_receipts).unwrap();
        let mut client = h.app.game_as(p.account);
        let original = before
            .class_state
            .successful_transfer_receipts
            .iter()
            .map(|r| change(&c, r.key, r.target_class_id.0))
            .collect::<Vec<_>>();
        let mut replies = Vec::new();
        for request in &original {
            replies.push(
                client
                    .change_class(request.clone())
                    .await
                    .unwrap()
                    .into_inner(),
            );
        }
        let mut socket = ws::join(&h.app, &p).await;
        socket.until(|m| spawned(m, &p)).await;
        let after = h.repo.get(c.id).await.unwrap().unwrap();
        assert_eq!(balances(&after), (expected, mask));
        assert_eq!(
            serde_json::to_vec(&after.class_state.successful_transfer_receipts).unwrap(),
            frozen_bytes
        );
        for (request, reply) in original.iter().zip(&replies) {
            assert_eq!(
                client
                    .change_class(request.clone())
                    .await
                    .unwrap()
                    .into_inner(),
                *reply
            );
        }
        assert_eq!(grants(&h.pool, &c).await.len(), grant_count);
        socket.close().await;
    }
    h.running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

fn event(message: &pb::ServerMessage) -> Option<&pb::world_event::Event> {
    match &message.payload {
        Some(pb::server_message::Payload::Event(pb::WorldEvent { event: Some(event) })) => {
            Some(event)
        },
        _ => None,
    }
}
async fn attack_target(socket: &mut ws::Ws, seq: u32, target: &str) {
    socket
        .send(&pb::ClientMessage {
            seq,
            intent: Some(pb::client_message::Intent::SetTarget(pb::SetTargetRequest {
                entity_id: target.into(),
            })),
        })
        .await;
    socket
        .send(&pb::ClientMessage {
            seq: seq.checked_add(1).unwrap(),
            intent: Some(pb::client_message::Intent::Attack(pb::AttackRequest {})),
        })
        .await;
}
async fn npc(h: &Harness, socket: &mut ws::Ws, name: &str, raw_xp: u64, lethal: bool) -> String {
    use nightfall_api::domain::zone::{Scaled, Speed, Vec2Fixed, ZoneCommand, ZoneInput};
    let mut combat = common::keltir();
    combat.template = name.into();
    combat.xp_reward = raw_xp;
    combat.stats.max_hp = if lethal { 1_000_000 } else { 1 };
    if lethal {
        combat.stats.p_atk = Scaled::from_raw(50_000_000_000);
    }
    h.app
        .zone
        .send(ZoneInput::system(ZoneCommand::SpawnNpc {
            name: name.into(),
            pos: Vec2Fixed::from_tiles(126, 126),
            speed: Speed::DEFAULT,
            combat: Some(Box::new(combat)),
        }))
        .unwrap();
    let frames = socket
        .until(|m| ws::spawn_of(m).is_some_and(|s| s.name == name))
        .await;
    ws::spawn_of(frames.last().unwrap())
        .unwrap()
        .entity_id
        .clone()
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Independent XP/death/rank oracle across actual combat crossings.
async fn real_combat_crossings_death_recross_and_one_kill_multilevel_jump_checkpoint_once_only_tokens(
) {
    use nightfall_api::domain::zone::{EntityId, ZoneCommand, ZoneInput};
    // Literal source-derived oracle shared with native fixture author; neither the
    // expected racial award nor death loss is computed by production helpers here.
    for (name, start, raw, award, destination, first_xp, death_xp, recross_xp, initial, mask) in [
        (
            "CombatTwenty",
            19,
            10235,
            10746,
            20,
            846_607,
            Some(832_278),
            Some(843_024),
            [0, 0],
            0,
        ),
        (
            "CombatForty",
            39,
            62751,
            65888,
            40,
            15_488_816,
            Some(15_400_965),
            Some(15_466_853),
            [1, 0],
            1,
        ),
        ("CombatJump", 19, 13_892_446, 14_587_068, 40, 15_422_929, None, None, [0, 0], 0),
    ] {
        let h = Harness::new().await;
        let mut c = character(name, start, initial, mask);
        c.xp = if start == 19 { 835_861 } else { 15_422_928 };
        let p = h.seed(&c).await;
        let mut socket = ws::join(&h.app, &p).await;
        socket.until(|m| spawned(m, &p)).await;
        let target = npc(&h, &mut socket, "Oracle", raw, false).await;
        attack_target(&mut socket, 1, &target).await;
        let frames=socket.until(|m|matches!(event(m),Some(pb::world_event::Event::LevelUp(up)) if up.level==destination)).await;
        assert!(frames.iter().any(|m|matches!(event(m),Some(pb::world_event::Event::XpGained(xp)) if xp.amount==award && xp.total==first_xp)));
        let saved = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!((saved.level, saved.xp), (destination, first_xp));
        let expected = if destination == 20 { [1, 0] } else { [1, 1] };
        let claimed = if destination == 20 { 1 } else { 3 };
        assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), (expected, claimed));
        let ranks = saved.class_state.learned_skills.clone();
        assert_eq!(
            ranks
                .iter()
                .map(|s| (s.key.as_str(), s.level))
                .collect::<Vec<_>>(),
            vec![
                ("l2.skill.1320", if destination == 20 { 2 } else { 4 }),
                ("l2.skill.1322", 1),
                ("l2.skill.194", 1),
                ("l2.skill.239", if destination == 20 { 1 } else { 2 })
            ]
        );
        let facts = grants(&h.pool, &c).await;
        assert_eq!(facts.len(), if name == "CombatJump" { 2 } else { 1 });
        for fact in &facts {
            assert_eq!(fact["source"], "LevelUp");
        }
        if let (Some(dead_xp), Some(final_xp)) = (death_xp, recross_xp) {
            let brute = npc(&h, &mut socket, "Sentinel", 0, true).await;
            attack_target(&mut socket, 3, &brute).await;
            socket.until(|m|matches!(event(m),Some(pb::world_event::Event::EntityDied(dead)) if dead.entity==p.entity_id())).await;
            let dead = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
            assert_eq!(
                (dead.level, dead.xp, dead.hp, dead.alive),
                (destination.checked_sub(1).unwrap(), dead_xp, Some(0), false)
            );
            assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), (expected, claimed));
            assert_eq!(dead.class_state.learned_skills, ranks);
            h.app
                .zone
                .send(ZoneInput::system(ZoneCommand::Despawn {
                    entity: EntityId::from_uuid(Uuid::parse_str(&brute).unwrap()),
                }))
                .unwrap();
            socket
                .send(&pb::ClientMessage {
                    seq: 5,
                    intent: Some(pb::client_message::Intent::Respawn(pb::RespawnRequest {})),
                })
                .await;
            socket.until(|m|matches!(event(m),Some(pb::world_event::Event::EntityRespawned(r)) if r.entity==p.entity_id())).await;
            assert_eq!(h.repo.load_for_admission(c.id).await.unwrap().unwrap().xp, dead_xp);
            let target = npc(&h, &mut socket, "RecrossOracle", raw, false).await;
            attack_target(&mut socket, 6, &target).await;
            socket.until(|m|matches!(event(m),Some(pb::world_event::Event::LevelUp(up)) if up.level==destination)).await;
            let recross = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
            assert_eq!((recross.level, recross.xp), (destination, final_xp));
            assert_eq!(recross.class_state.learned_skills, ranks);
            assert_eq!(grants(&h.pool, &c).await, facts);
            assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), (expected, claimed));
        }
        // Stop the attack normally and wait for the recorded swing cooldown; a
        // death/kill frame precedes ready_at and does not make a transfer eligible.
        socket
            .send(&pb::ClientMessage {
                seq: 8,
                intent: Some(pb::client_message::Intent::StopAttack(pb::StopAttackRequest {})),
            })
            .await;
        socket
            .until(|m| ws::ack(m).is_some_and(|ack| ack.seq == 8))
            .await;
        let snapshot = h.app.zone.snapshot().await.unwrap();
        let ready = snapshot
            .entities
            .iter()
            .find(|e| e.id.as_uuid() == c.id.as_uuid())
            .unwrap()
            .combat
            .as_ref()
            .unwrap()
            .ready_at;
        let mut clock = h.app.zone.stats();
        tokio::time::timeout(ws::WAIT, clock.wait_for(|stats| stats.tick >= ready))
            .await
            .unwrap()
            .unwrap();
        // These are normal real gRPC transfers at the Master after combat cools down.
        let mut client = h.app.game_as(p.account);
        let first_request = change(&c, Uuid::now_v7(), 1);
        let first = client
            .change_class(first_request.clone())
            .await
            .unwrap()
            .into_inner();
        assert!(first.granted_skill_keys.is_empty());
        if destination == 40 {
            let second = client
                .change_class(change(&c, Uuid::now_v7(), 2))
                .await
                .unwrap()
                .into_inner();
            assert!(second.granted_skill_keys.is_empty());
        }
        let consumed = h.repo.get(c.id).await.unwrap().unwrap();
        assert_eq!(balances(&consumed), ([0, 0], claimed));
        let mut replacement = ws::join(&h.app, &p).await;
        replacement.until(|m| spawned(m, &p)).await;
        assert_eq!(socket.closed(ws::WAIT).await, Some(4409));
        assert_eq!(h.repo.get(c.id).await.unwrap().unwrap().class_state, consumed.class_state);
        assert_eq!(
            client
                .change_class(first_request.clone())
                .await
                .unwrap()
                .into_inner(),
            first
        );
        assert_eq!(grants(&h.pool, &c).await, facts);
        if death_xp.is_some() && recross_xp.is_some() {
            let (spent_dead_xp, spent_recross_xp) = if destination == 20 {
                (828_695, 839_441)
            } else {
                (15_379_002, 15_444_890)
            };
            let frozen =
                serde_json::to_vec(&consumed.class_state.successful_transfer_receipts).unwrap();
            let grant_metrics = token_metrics(&h);
            assert_eq!(grant_metrics.len(), 1);
            assert!(grant_metrics[0].contains("source=\"level_up\""));
            assert!(grant_metrics[0].ends_with(" 1"));
            let brute = npc(&h, &mut replacement, "SpentSentinel", 0, true).await;
            attack_target(&mut replacement, 1, &brute).await;
            replacement.until(|m| matches!(event(m), Some(pb::world_event::Event::EntityDied(dead)) if dead.entity == p.entity_id())).await;
            let dead = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
            assert_eq!(
                (dead.level, dead.xp, dead.hp, dead.alive),
                (destination.checked_sub(1).unwrap(), spent_dead_xp, Some(0), false)
            );
            assert_eq!(dead.class_state.current_class_id, consumed.class_state.current_class_id);
            assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), ([0, 0], claimed));
            assert_eq!(
                serde_json::to_vec(&dead.class_state.successful_transfer_receipts).unwrap(),
                frozen
            );
            assert_eq!(grants(&h.pool, &c).await, facts);
            assert_eq!(token_metrics(&h), grant_metrics);
            h.app
                .zone
                .send(ZoneInput::system(ZoneCommand::Despawn {
                    entity: EntityId::from_uuid(Uuid::parse_str(&brute).unwrap()),
                }))
                .unwrap();
            replacement
                .send(&pb::ClientMessage {
                    seq: 3,
                    intent: Some(pb::client_message::Intent::Respawn(pb::RespawnRequest {})),
                })
                .await;
            replacement.until(|m| matches!(event(m), Some(pb::world_event::Event::EntityRespawned(r)) if r.entity == p.entity_id())).await;
            assert_eq!(h.repo.load_for_admission(c.id).await.unwrap().unwrap().xp, spent_dead_xp);
            let target = npc(&h, &mut replacement, "SpentRecrossOracle", raw, false).await;
            attack_target(&mut replacement, 4, &target).await;
            replacement.until(|m| matches!(event(m), Some(pb::world_event::Event::LevelUp(up)) if up.level == destination)).await;
            let relevel = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
            assert_eq!((relevel.level, relevel.xp), (destination, spent_recross_xp));
            assert_eq!(relevel.class_state.current_class_id, consumed.class_state.current_class_id);
            assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), ([0, 0], claimed));
            assert_eq!(
                serde_json::to_vec(&relevel.class_state.successful_transfer_receipts).unwrap(),
                frozen
            );
            assert_eq!(grants(&h.pool, &c).await, facts);
            assert_eq!(token_metrics(&h), grant_metrics);
            assert_eq!(
                client
                    .change_class(first_request.clone())
                    .await
                    .unwrap()
                    .into_inner(),
                first
            );
        }
        replacement.close().await;
        h.running.shutdown(WatermarkReason::Shutdown).await.unwrap();
    }
}

fn token_metrics(h: &Harness) -> Vec<String> {
    h.metrics
        .render()
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("nightfall_class_transfer_token_grants_total{"))
        .map(str::to_owned)
        .collect()
}
