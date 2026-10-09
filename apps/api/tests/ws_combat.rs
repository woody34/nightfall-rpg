//! Combat through real sockets: E2.1 contract and intent plumbing, E2.3 auto-attack and
//! damage, E2.2 AOI privacy of combat facts (Respawn stays a stub until E2.4).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::float_cmp
)]
mod common;

use common::ws::{ack, join, move_of, rejected, seed_player, spawn_of, stop_move, WAIT};
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
/// The fixture zone with one NPC at `(x, 10)`: a Keltir combatant when `attackable`, else a
/// noncombat fixture; `dead` leaves it a corpse.
fn npc_zone(dead: bool, attackable: bool, x: i32) -> (ZoneState, EntityId) {
    let mut z = common::fixture_zone();
    let draft = z.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
        name: "Target".into(),
        pos: Vec2Fixed::from_tiles(x, 10),
        speed: Speed::DEFAULT,
        combat: attackable.then(|| Box::new(common::keltir())),
    })]);
    z.run_tick(draft).unwrap();
    let mut snapshot = z.snapshot();
    let npc = snapshot
        .entities
        .iter_mut()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap();
    if dead {
        npc.targeting.dead = true;
        npc.combat.as_mut().unwrap().hp = 0;
    }
    let id = npc.id;
    (ZoneState::from_snapshot(snapshot).unwrap(), id)
}

/// Joins and reads through the admission tick (own spawn, AOI, own `StatsChanged`).
async fn enter(app: &TestApp, p: &common::ws::Player) -> common::ws::Ws {
    let mut ws = join(app, p).await;
    ws.until(|m| stats(m).is_some()).await;
    ws
}

fn event(msg: &pb::ServerMessage) -> Option<&Event> {
    match &msg.payload {
        Some(Payload::Event(pb::WorldEvent { event: Some(e) })) => Some(e),
        _ => None,
    }
}
fn stats(msg: &pb::ServerMessage) -> Option<&pb::StatsChanged> {
    match event(msg) {
        Some(Event::StatsChanged(s)) => Some(s),
        _ => None,
    }
}
fn result(msg: &pb::ServerMessage) -> Option<&pb::AttackResult> {
    match event(msg) {
        Some(Event::AttackResult(r)) => Some(r),
        _ => None,
    }
}
fn started(msg: &pb::ServerMessage) -> Option<&pb::AttackStarted> {
    match event(msg) {
        Some(Event::AttackStarted(s)) => Some(s),
        _ => None,
    }
}
fn died(msg: &pb::ServerMessage) -> Option<&pb::EntityDied> {
    match event(msg) {
        Some(Event::EntityDied(d)) => Some(d),
        _ => None,
    }
}
fn stop_attack(seq: u32) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(Intent::StopAttack(pb::StopAttackRequest {})),
    }
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
        let mut ws = enter(&app, &p).await;
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
    let mut ws = enter(&app, &p).await;
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
    let mut wa = enter(&app, &a).await;
    let _wb = enter(&app, &b).await;
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
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = enter(&app, &p).await;
    let mut ticks = app.zone.subscribe();
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
    // Attack twice is accepted twice and inert the second time; StopMove ends the attack
    // before a swing starts; StopAttack is then a no-op; Respawn is still a stub (E2.4).
    assert_eq!(
        responses,
        vec![
            (1, None),
            (2, None),
            (3, None),
            (4, Some(3)),
            (5, None),
            (6, None),
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
    let mut ws = enter(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
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
    let mut old = enter(&app, &p).await;
    old.send(&target(1, npc)).await;
    old.until(|m| changed(m).is_some()).await;
    let mut new = join(&app, &p).await;
    assert_eq!(old.closed(WAIT).await, Some(4409));
    // The replacement is told the AOI, its stats and its selection again.
    let msgs = new
        .until(|m| changed(m).is_some_and(|t| t.target == npc.to_string()))
        .await;
    assert!(msgs
        .iter()
        .any(|m| spawn_of(m).is_some_and(|s| s.entity_id == npc.to_string() && s.combatant)));
    assert!(msgs.iter().any(|m| stats(m).is_some()));
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

// ---- E2.3 auto-attack and damage, E2.2 privacy -------------------------------------------------

/// Like [`npc_zone`] for a living Keltir, with its HP and P.Atk overridden.
fn keltir_zone(x: i32, hp: Option<u32>, p_atk: Option<i64>) -> (ZoneState, EntityId) {
    let mut z = common::fixture_zone();
    let mut keltir = common::keltir();
    if let Some(a) = p_atk {
        keltir.stats.p_atk = nightfall_api::domain::zone::Scaled::from_int(a).unwrap();
    }
    let draft = z.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
        name: "Keltir".into(),
        pos: Vec2Fixed::from_tiles(x, 10),
        speed: Speed::from_milli_tiles_per_tick(400),
        combat: Some(Box::new(keltir)),
    })]);
    z.run_tick(draft).unwrap();
    let mut snapshot = z.snapshot();
    let npc = snapshot
        .entities
        .iter_mut()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap();
    if let Some(hp) = hp {
        npc.combat.as_mut().unwrap().hp = hp;
    }
    let id = npc.id;
    (ZoneState::from_snapshot(snapshot).unwrap(), id)
}

#[tokio::test]
async fn attack_chases_then_swings_on_server_ticks_with_server_damage() {
    let (zone, npc) = keltir_zone(13, None, None);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = enter(&app, &p).await;
    let mut ticks = app.zone.subscribe();
    ws.send(&target(1, npc)).await;
    // An Attack frame carrying an unknown "damage" field: prost drops it, and nothing a
    // client sends can name damage, HP or rolls.
    ws.send_raw(vec![0x08, 0x02, 0x6a, 0x03, 0x08, 0x8f, 0x4e])
        .await;
    let msgs = ws
        .until(|m| started(m).is_some_and(|s| s.attacker == p.entity_id()))
        .await;
    assert!(msgs.iter().any(|m| ack(m).is_some_and(|a| a.seq == 2)));
    assert!(
        msgs.iter()
            .any(|m| move_of(m).is_some_and(|mv| mv.entity_id == p.entity_id())),
        "3 tiles away: the player walks into reach first"
    );
    let s = started(msgs.last().unwrap()).unwrap().clone();
    assert_eq!(s.target, npc.to_string());
    assert_eq!((s.impact_tick - s.tick, s.ready_tick - s.tick), (6, 12));
    let msgs = ws
        .until(|m| result(m).is_some_and(|r| r.attacker == p.entity_id()))
        .await;
    let r = result(msgs.last().unwrap()).unwrap().clone();
    assert_eq!(r.tick, s.impact_tick);
    assert_eq!(r.target_hp_after, 44 - r.damage);
    // The wire fact is the zone's fact.
    let mut seen = None;
    while let Ok(t) = ticks.try_recv() {
        for e in &t.events {
            if let ZoneEvent::AttackResult {
                attacker,
                damage,
                target_hp_after,
                ..
            } = e
            {
                if attacker.to_string() == p.entity_id() {
                    seen.get_or_insert((*damage, *target_hp_after));
                }
            }
        }
    }
    assert_eq!(seen, Some((r.damage, r.target_hp_after)));
    // Read after write: the zone holds the same HP.
    let snap = app.zone.snapshot().await.unwrap();
    let monster = snap.entities.iter().find(|e| e.id == npc).unwrap();
    assert_eq!(monster.combat.as_ref().unwrap().hp, r.target_hp_after);
}

#[tokio::test]
async fn repeated_attack_adds_no_swing_and_stop_attack_ends_the_cycle() {
    let (zone, npc) = keltir_zone(11, Some(10_000), None);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = enter(&app, &p).await;
    ws.send(&target(1, npc)).await;
    ws.send(&attack(2)).await;
    let first = ws
        .until(|m| started(m).is_some_and(|s| s.attacker == p.entity_id()))
        .await;
    let first = started(first.last().unwrap()).unwrap().tick;
    ws.send(&attack(3)).await;
    ws.send(&attack(4)).await;
    let mut starts = vec![first];
    let msgs = ws.drain(Duration::from_millis(2_600)).await;
    starts.extend(
        msgs.iter()
            .filter_map(started)
            .filter(|s| s.attacker == p.entity_id())
            .map(|s| s.tick),
    );
    assert!(msgs.iter().any(|m| ack(m).is_some_and(|a| a.seq == 4)));
    assert!(starts.len() >= 2);
    assert!(starts.windows(2).all(|w| w[1] - w[0] == 12), "{starts:?}");
    ws.send(&stop_attack(5)).await;
    ws.until(|m| ack(m).is_some_and(|a| a.seq == 5)).await;
    let after = ws.drain(Duration::from_millis(1_600)).await;
    assert!(after
        .iter()
        .filter_map(started)
        .all(|s| s.attacker != p.entity_id()));
    let snap = app.zone.snapshot().await.unwrap();
    let me = snap
        .entities
        .iter()
        .find(|e| e.id.to_string() == p.entity_id())
        .unwrap();
    assert!(!me.combat.as_ref().unwrap().auto_attack);
    assert_eq!(me.targeting.target, Some(npc), "stopping keeps the selection");
}

#[tokio::test]
async fn combat_facts_reach_observers_who_know_both_sides_and_stats_only_the_owner() {
    let (zone, npc) = keltir_zone(11, Some(10_000), None);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let a = seed_player(&app, "Alpha", 10.0, 10.0);
    let b = seed_player(&app, "Bravo", 12.0, 10.0);
    let c = seed_player(&app, "Charlie", 200.0, 200.0);
    let mut wa = enter(&app, &a).await;
    let mut wb = enter(&app, &b).await;
    let mut wc = enter(&app, &c).await;
    wa.send(&target(1, npc)).await;
    wa.send(&attack(2)).await;
    wa.until(|m| result(m).is_some_and(|r| r.attacker == a.entity_id()))
        .await;
    let seen_by_b = wb
        .until(|m| result(m).is_some_and(|r| r.attacker == a.entity_id()))
        .await;
    assert!(seen_by_b
        .iter()
        .any(|m| started(m).is_some_and(|s| s.attacker == a.entity_id())));
    assert!(seen_by_b
        .iter()
        .filter_map(stats)
        .all(|s| s.entity == b.entity_id()));
    assert!(seen_by_b
        .iter()
        .filter_map(changed)
        .all(|t| t.entity == b.entity_id()));
    let far = wc.drain(Duration::from_millis(400)).await;
    assert!(far
        .iter()
        .all(|m| result(m).is_none() && started(m).is_none()));
}

#[tokio::test]
async fn a_kill_is_announced_once_clears_the_target_and_a_dead_player_is_refused() {
    // One hit kills the Keltir; a second, deadly Keltir then kills the player.
    let (zone, npc) = keltir_zone(11, Some(1), None);
    let app = TestApp::spawn_with_zone(|_| {}, zone).await;
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = enter(&app, &p).await;
    ws.send(&target(1, npc)).await;
    ws.send(&attack(2)).await;
    let mut seq = 2;
    // Misses are possible; keep the attack going until the kill.
    let msgs = ws.until(|m| died(m).is_some()).await;
    let d = died(msgs.last().unwrap()).unwrap().clone();
    assert_eq!(
        (d.entity.as_str(), d.killer.as_str(), d.incarnation),
        (npc.to_string().as_str(), p.entity_id().as_str(), 1)
    );
    let after = ws.until(|m| changed(m).is_some()).await;
    assert_eq!(changed(after.last().unwrap()).unwrap().target, "");
    seq += 1;
    ws.send(&target(seq, npc)).await;
    let r = ws.until(|m| rejected(m).is_some()).await;
    assert_eq!(
        rejected(r.last().unwrap()).unwrap().reason,
        i32::from(RejectReason::NonAttackableTarget)
    );
    assert!(
        ws.drain(Duration::from_millis(300))
            .await
            .iter()
            .all(|m| died(m).is_none()),
        "announced once"
    );

    let mut killer = common::keltir();
    killer.stats.p_atk = nightfall_api::domain::zone::Scaled::from_int(100_000).unwrap();
    app.zone
        .send(ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Brute".into(),
            pos: Vec2Fixed::from_tiles(11, 11),
            speed: Speed::DEFAULT,
            combat: Some(Box::new(killer)),
        }))
        .unwrap();
    let spawned = ws
        .until(|m| spawn_of(m).is_some_and(|s| s.name == "Brute"))
        .await;
    let brute = spawn_of(spawned.last().unwrap()).unwrap().entity_id.clone();
    let me = EntityId::from_uuid(p.character.as_uuid());
    app.zone
        .send(ZoneInput::system(ZoneCommand::AddAggro {
            npc: EntityId::from_uuid(Uuid::parse_str(&brute).unwrap()),
            target: me,
        }))
        .unwrap();
    let msgs = ws
        .until(|m| died(m).is_some_and(|d| d.entity == p.entity_id()))
        .await;
    let hit = msgs
        .iter()
        .filter_map(result)
        .find(|r| r.target == p.entity_id() && r.damage > 0)
        .unwrap();
    assert_eq!(hit.target_hp_after, 0);
    assert!(msgs
        .iter()
        .filter_map(stats)
        .any(|s| s.entity == p.entity_id() && s.hp == 0));
    for msg in [attack(seq + 1), stop_move(seq + 2), target(seq + 3, "")] {
        let want = msg.seq;
        ws.send(&msg).await;
        let r = ws
            .until(|m| rejected(m).is_some_and(|r| r.seq == want))
            .await;
        assert_eq!(rejected(r.last().unwrap()).unwrap().reason, i32::from(RejectReason::DeadActor));
    }
}
