//! NPC AI through real sockets and real 100 ms ticks (Phase 1 E3.2–E3.4): two players walk
//! into two spawn-slot Keltirs, are auto/socially aggroed, kill them, see the corpses decay,
//! get `UNKNOWN_ENTITY` for the hidden old life and see each slot respawn a new incarnation.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::float_cmp
)]
mod common;

use common::ws::{join, rejected, seed_player, spawn_of, Player, Ws};
use common::TestApp;
use nightfall_api::application::zone_bootstrap::slot_specs;
use nightfall_api::domain::zone::{Scaled, ZoneState};
use nightfall_api::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};
use nightfall_api::interface::grpc::pb::{
    self, client_message::Intent, server_message::Payload, world_event::Event, ClientMessage,
    RejectReason,
};

/// The fixture zone with the test zone's two Keltir slots, one member each: harmless
/// (P.Atk 1, so no player dies before E2.4), corpses for 1 s, respawn 3 s after death.
fn keltir_zone() -> ZoneState {
    let mut def = parse_zone(TEST_ZONE_TOML).unwrap();
    for slot in &mut def.spawn_slots {
        slot.count = 1;
        slot.respawn_delay_secs = 3;
        slot.respawn_random_secs = 0;
    }
    let mut specs = slot_specs(&common::rules(), &def).unwrap();
    for s in &mut specs {
        s.combat.stats.p_atk = Scaled::from_int(1).unwrap();
        s.brain.corpse_decay_ticks = 10;
    }
    common::fixture_zone().with_spawn_slots(specs)
}

fn event(m: &pb::ServerMessage) -> Option<&Event> {
    match &m.payload {
        Some(Payload::Event(pb::WorldEvent { event: Some(e) })) => Some(e),
        _ => None,
    }
}

fn target(seq: u32, id: &str) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(Intent::SetTarget(pb::SetTargetRequest {
            entity_id: id.to_owned(),
        })),
    }
}

fn attack(seq: u32) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(Intent::Attack(pb::AttackRequest {})),
    }
}

/// Joins and returns the Keltir spawns of the admission tick.
async fn enter(app: &TestApp, p: &Player) -> (Ws, Vec<pb::EntitySpawn>) {
    let mut ws = join(app, p).await;
    let msgs = ws
        .until(|m| matches!(event(m), Some(Event::StatsChanged(_))))
        .await;
    let keltirs = msgs
        .iter()
        .filter_map(spawn_of)
        .filter(|s| s.template_id == "keltir")
        .cloned()
        .collect();
    (ws, keltirs)
}

/// One player's fight: wait until a Keltir lands an attack on someone (auto or clan aggro;
/// a clan call may send both at the same player), kill `mine`, then see its corpse leave, get `UNKNOWN_ENTITY` for it while hidden, and see the new
/// life arrive.
async fn fight(mut ws: Ws, me: String, mine: String, keltirs: [String; 2]) {
    ws.until(|m| matches!(event(m), Some(Event::AttackResult(r)) if keltirs.contains(&r.attacker)))
        .await;
    ws.send(&target(1, &mine)).await;
    ws.send(&attack(2)).await;
    let msgs = ws
        .until(|m| matches!(event(m), Some(Event::EntityDied(d)) if d.entity == mine))
        .await;
    let Some(Event::EntityDied(d)) = event(msgs.last().unwrap()) else {
        unreachable!()
    };
    assert_eq!((d.killer.as_str(), d.incarnation), (me.as_str(), 1));
    ws.until(|m| matches!(&m.payload, Some(Payload::Event(pb::WorldEvent { event: Some(Event::Despawn(x)) })) if x.entity_id == mine))
        .await;
    ws.send(&target(3, &mine)).await;
    let r = ws.until(|m| rejected(m).is_some_and(|r| r.seq == 3)).await;
    assert_eq!(
        rejected(r.last().unwrap()).unwrap().reason,
        i32::from(RejectReason::UnknownEntity),
        "the hidden old life is unknown"
    );
    let msgs = ws
        .until(|m| spawn_of(m).is_some_and(|s| s.entity_id == mine))
        .await;
    let s = spawn_of(msgs.last().unwrap()).unwrap();
    assert_eq!((s.life_incarnation, s.dead, s.hp, s.attackable), (2, false, s.max_hp, true));
    assert!(ws
        .until(|m| matches!(event(m), Some(Event::EntityRespawned(r)) if r.entity == mine))
        .await
        .last()
        .is_some());
}

#[tokio::test]
async fn two_players_fight_two_keltirs_through_aggro_death_and_respawn() {
    let app = TestApp::spawn_with_zone(|_| {}, keltir_zone()).await;
    // Alpha 2 tiles from slot a (100, 100), Bravo 2 tiles from slot b (104, 102); each is
    // outside the other Keltir's 6-tile aggro range, so the second one engages by clan call
    // or by its own think, whichever comes first.
    let alpha = seed_player(&app, "Alpha", 98.0, 100.0);
    let bravo = seed_player(&app, "Bravo", 106.0, 102.0);
    let (wa, ka) = enter(&app, &alpha).await;
    let (wb, kb) = enter(&app, &bravo).await;
    assert_eq!(ka.len(), 2);
    assert_eq!(kb.len(), 2);
    let west = |ks: &[pb::EntitySpawn]| {
        let mut ks = ks.to_vec();
        ks.sort_by(|a, b| {
            let (ax, bx) = (a.position.as_ref().unwrap().x, b.position.as_ref().unwrap().x);
            ax.total_cmp(&bx)
        });
        (ks[0].entity_id.clone(), ks[1].entity_id.clone())
    };
    let (a_keltir, b_keltir) = west(&ka);
    assert_eq!(west(&kb), (a_keltir.clone(), b_keltir.clone()));
    assert!(ka
        .iter()
        .all(|k| k.life_incarnation == 1 && k.attackable && !k.dead));
    let both = [a_keltir.clone(), b_keltir.clone()];
    tokio::join!(
        fight(wa, alpha.entity_id(), a_keltir, both.clone()),
        fight(wb, bravo.entity_id(), b_keltir, both),
    );
}
