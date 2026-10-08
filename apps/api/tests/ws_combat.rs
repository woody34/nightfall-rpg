//! E2.1 contract and intent plumbing through real sockets (combat execution is deferred).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::float_cmp
)]
mod common;

use common::ws::{ack, join, rejected, seed_player, spawn_of, stop_move, WAIT};
use common::TestApp;
use futures_util::SinkExt as _;
use nightfall_api::application::replay_log::AppliedTickRecord;
use nightfall_api::domain::zone::{
    EntityId, EntityKind, SessionGeneration, Speed, Vec2Fixed, ZoneCommand, ZoneEvent, ZoneInput,
    ZoneState,
};
use nightfall_api::interface::grpc::pb::{
    self, client_message::Intent, server_message::Payload, world_event::Event, ClientMessage,
    RejectReason,
};
use nightfall_api::interface::zone_mapping::encode_observer_outputs;
use prost::Message as _;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[allow(clippy::needless_pass_by_value)] // accepts UUIDs, entity IDs and literal clear/malformed targets
fn target(seq: u32, id: impl ToString) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(Intent::SetTarget(pb::SetTargetRequest {
            entity_id: id.to_string(),
        })),
    }
}
fn attack(seq: u32) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(Intent::Attack(pb::AttackRequest {})),
    }
}
fn changed(msg: &pb::ServerMessage) -> Option<&pb::TargetChanged> {
    match &msg.payload {
        Some(Payload::Event(pb::WorldEvent {
            event: Some(Event::TargetChanged(t)),
        })) => Some(t),
        _ => None,
    }
}
fn npc_zone(dead: bool, attackable: bool, x: i32) -> (ZoneState, EntityId) {
    let mut z = common::fixture_zone();
    let draft = z.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
        name: "Target".into(),
        pos: Vec2Fixed::from_tiles(x, 10),
        speed: Speed::DEFAULT,
    })]);
    z.run_tick(draft).unwrap();
    let mut snapshot = z.snapshot();
    let npc = snapshot
        .entities
        .iter_mut()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap();
    npc.targeting.dead = dead;
    npc.targeting.attackable = attackable;
    let id = npc.id;
    (ZoneState::from_snapshot(snapshot).unwrap(), id)
}

#[tokio::test]
async fn set_target_unknown_dead_nonattackable_and_out_of_aoi_leave_selection_unchanged() {
    for (dead, attackable, x, expected) in [
        (true, true, 11, RejectReason::NonAttackableTarget),
        (false, false, 11, RejectReason::NonAttackableTarget),
        (false, true, 200, RejectReason::TargetNotInAoi),
    ] {
        let (zone, npc) = npc_zone(dead, attackable, x);
        let app = TestApp::spawn_with_zone(|_| {}, zone).await;
        let p = seed_player(&app, "Alpha", 10.0, 10.0);
        let mut ws = join(&app, &p).await;
        let mut ticks = app.zone.subscribe();
        for (seq, id, reason) in [
            (1, "malformed".into(), RejectReason::Invalid),
            (2, Uuid::from_u128(999).to_string(), RejectReason::UnknownEntity),
            (3, npc.to_string(), expected),
        ] {
            ws.send(&target(seq, id)).await;
            let msgs = ws
                .until(|m| rejected(m).is_some_and(|r| r.seq == seq))
                .await;
            let r = rejected(msgs.last().unwrap()).unwrap();
            assert_eq!(r.reason, i32::from(reason));
            assert!(!r.detail.is_empty());
            assert!(msgs.iter().all(|m| changed(m).is_none()));
        }
        let snap = app.zone.snapshot().await.unwrap();
        let actor = snap
            .entities
            .iter()
            .find(|e| e.id.as_uuid() == p.character.as_uuid())
            .unwrap();
        assert_eq!(actor.targeting.target, None);
        while let Ok(t) = ticks.try_recv() {
            assert!(t
                .events
                .iter()
                .all(|e| !matches!(e, ZoneEvent::TargetChanged { .. })));
        }
    }
}

#[tokio::test]
async fn set_target_select_repeat_clear_and_read_after_write() {
    let (zone, npc) = npc_zone(false, true, 11);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = join(&app, &p).await;
    ws.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == npc.to_string()))
        .await;
    ws.send(&target(1, npc)).await;
    let msgs = ws.until(|m| changed(m).is_some()).await;
    assert_eq!(ack(&msgs[0]).unwrap().seq, 1);
    assert_eq!(
        changed(msgs.last().unwrap()).unwrap(),
        &pb::TargetChanged {
            entity: p.entity_id(),
            target: npc.to_string()
        }
    );
    let snapshot = app.zone.snapshot().await.unwrap();
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id.as_uuid() == p.character.as_uuid())
            .unwrap()
            .targeting
            .target,
        Some(npc)
    );
    ws.send(&target(2, npc)).await;
    assert_eq!(ack(&ws.next_msg().await).unwrap().seq, 2);
    assert!(ws.drain(Duration::from_millis(150)).await.is_empty());
    ws.send(&target(3, "")).await;
    let msgs = ws.until(|m| changed(m).is_some()).await;
    assert_eq!(ack(&msgs[0]).unwrap().seq, 3);
    assert_eq!(changed(msgs.last().unwrap()).unwrap().target, "");
    ws.send(&target(4, "")).await;
    assert_eq!(ack(&ws.next_msg().await).unwrap().seq, 4);
    assert!(ws.drain(Duration::from_millis(150)).await.is_empty());
}

#[tokio::test]
async fn player_target_is_nonattackable_and_cannot_choose_the_actor_identity() {
    let app = TestApp::spawn().await;
    let a = seed_player(&app, "Alpha", 10.0, 10.0);
    let b = seed_player(&app, "Bravo", 11.0, 10.0);
    let mut wa = join(&app, &a).await;
    let _wb = join(&app, &b).await;
    wa.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == b.entity_id()))
        .await;
    wa.send(&target(1, b.entity_id())).await;
    let msgs = wa.until(|m| rejected(m).is_some()).await;
    assert_eq!(
        rejected(msgs.last().unwrap()).unwrap().reason,
        i32::from(RejectReason::NonAttackableTarget)
    );
    assert!(app
        .zone
        .snapshot()
        .await
        .unwrap()
        .entities
        .iter()
        .all(|e| e.targeting.target.is_none()));
}

#[tokio::test]
async fn mixed_intents_preserve_actor_stream_and_repeated_attack_is_inert_and_recorded() {
    let (zone, npc) = npc_zone(false, true, 11);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let mut ticks = app.zone.subscribe();
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = join(&app, &p).await;
    let messages = [
        target(1, npc),
        attack(2),
        attack(3),
        target(4, Uuid::from_u128(999)),
        stop_move(5),
        ClientMessage {
            seq: 6,
            intent: Some(Intent::StopAttack(pb::StopAttackRequest {})),
        },
        ClientMessage {
            seq: 7,
            intent: Some(Intent::Respawn(pb::RespawnRequest {})),
        },
    ];
    for msg in &messages {
        ws.socket
            .feed(Message::Binary(msg.encode_to_vec().into()))
            .await
            .unwrap();
    }
    ws.socket.flush().await.unwrap();
    let received = ws.until(|m| rejected(m).is_some_and(|r| r.seq == 7)).await;
    let mut received = received;
    received.extend(ws.drain(Duration::from_millis(200)).await);
    let me = EntityId::from_uuid(p.character.as_uuid());
    let mut expected = Vec::new();
    let mut attacks = 0;
    let mut applied = Vec::new();
    while let Ok(t) = ticks.try_recv() {
        attacks += t
            .commands
            .iter()
            .filter(|c| matches!(c.command, ZoneCommand::Attack { entity } if entity == me))
            .count();
        assert!(t
            .events
            .iter()
            .all(|e| !matches!(e, ZoneEvent::AttackResult { .. })));
        let record = AppliedTickRecord::from_applied(nightfall_api::domain::zone::ZoneId(1), &t);
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
        if let Some(out) = t.outputs.get(&me) {
            expected.extend(encode_observer_outputs(out, t.server_time_ms));
        }
        applied.extend(
            t.commands
                .iter()
                .filter_map(|c| c.seq.map(|seq| (seq, t.tick.0))),
        );
    }
    assert_eq!(attacks, 2);
    assert_eq!(
        received
            .iter()
            .map(prost::Message::encode_to_vec)
            .collect::<Vec<_>>(),
        expected.iter().map(|b| b.to_vec()).collect::<Vec<_>>()
    );
    let responses: Vec<_> = received
        .iter()
        .filter_map(|m| {
            ack(m)
                .map(|a| (a.seq, None))
                .or_else(|| rejected(m).map(|r| (r.seq, Some(r.reason))))
        })
        .collect();
    assert_eq!(
        responses,
        vec![
            (1, None),
            (2, Some(12)),
            (3, Some(12)),
            (4, Some(3)),
            (5, None),
            (6, Some(12)),
            (7, Some(12))
        ]
    );
    for a in received.iter().filter_map(ack) {
        assert!(applied.contains(&(a.seq, a.tick)));
    }
}

#[tokio::test]
async fn combat_intents_obey_seq_regression_rate_and_tick_budget() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
    ws.next_msg().await;
    let mut ticks = app.zone.subscribe();
    for seq in 1..=31 {
        ws.socket
            .feed(Message::Binary(attack(seq).encode_to_vec().into()))
            .await
            .unwrap();
    }
    ws.socket.flush().await.unwrap();
    let mut responses = Vec::new();
    while responses.len() < 31 {
        let msg = ws.next_msg().await;
        if let Some(r) = rejected(&msg) {
            responses.push((r.seq, r.reason));
        }
    }
    let limited: Vec<_> = responses
        .iter()
        .filter(|(_, r)| *r == i32::from(RejectReason::RateLimited))
        .collect();
    assert_eq!(limited, vec![&(31, i32::from(RejectReason::RateLimited))]);
    let mut count = 0;
    while let Ok(t) = ticks.try_recv() {
        let n = t
            .commands
            .iter()
            .filter(|c| matches!(c.command, ZoneCommand::Attack { .. }))
            .count();
        assert!(n <= 8);
        count += n;
    }
    assert_eq!(count, 30);
    ws.send(&attack(31)).await;
    assert_eq!(ws.closed(WAIT).await, Some(4400));
}

#[tokio::test]
async fn replacement_fences_old_combat_commands_and_keeps_new_socket_selection() {
    let (zone, npc) = npc_zone(false, true, 11);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut old = join(&app, &p).await;
    old.send(&target(1, npc)).await;
    old.until(|m| changed(m).is_some()).await;
    let mut new = join(&app, &p).await;
    assert_eq!(old.closed(WAIT).await, Some(4409));
    new.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == npc.to_string()))
        .await;
    let me = EntityId::from_uuid(p.character.as_uuid());
    let mut ticks = app.zone.subscribe();
    // Deterministically model already queued frames from the closed generation.
    for (seq, command) in [
        (
            2,
            ZoneCommand::SetTarget {
                entity: me,
                target: None,
            },
        ),
        (3, ZoneCommand::Attack { entity: me }),
        (4, ZoneCommand::StopAttack { entity: me }),
        (5, ZoneCommand::Respawn { entity: me }),
    ] {
        app.zone
            .send(ZoneInput::session(me, SessionGeneration(1), seq, command))
            .unwrap();
    }
    let mut refused = 0;
    while refused < 4 {
        let t = tokio::time::timeout(WAIT, ticks.recv())
            .await
            .unwrap()
            .unwrap();
        refused += t
            .dispositions
            .iter()
            .filter(|d| d.reason == nightfall_api::domain::zone::RejectReason::StaleSession)
            .count();
    }
    assert!(
        new.drain(Duration::from_millis(150)).await.is_empty(),
        "stale responses must not leak to replacement"
    );
    assert_eq!(
        app.zone
            .snapshot()
            .await
            .unwrap()
            .entities
            .iter()
            .find(|e| e.id == me)
            .unwrap()
            .targeting
            .target,
        Some(npc)
    );
    new.send(&target(1, "")).await;
    new.until(|m| changed(m).is_some_and(|t| t.target.is_empty()))
        .await;
}
