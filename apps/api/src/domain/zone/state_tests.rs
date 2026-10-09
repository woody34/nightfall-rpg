//! Unit tests for `ZoneState`. The fixture zone is 256x256 tiles: 8x8 AOI cells, so an
//! entity can be outside another's 3x3 neighbourhood.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use uuid::Uuid;

use super::*;

const GEN1: SessionGeneration = SessionGeneration(1);

fn zone() -> ZoneState {
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 7,
        },
        bounds,
        1_000_000,
    )
}

fn id(n: u128) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(n))
}

fn run(z: &mut ZoneState, inputs: Vec<ZoneInput>) -> AppliedTick {
    let draft = z.draft(inputs);
    z.run_tick(draft).unwrap()
}

fn spawn_player(n: u128, x: i32, y: i32) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: id(n),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::DEFAULT,
        generation: GEN1,
        load: None,
    })
}

fn move_to(n: u128, dest: Vec2Fixed) -> ZoneInput {
    ZoneInput::session(
        id(n),
        GEN1,
        42,
        ZoneCommand::MoveTo {
            entity: id(n),
            dest,
        },
    )
}

fn reasons(t: &AppliedTick) -> Vec<RejectReason> {
    t.dispositions.iter().map(|d| d.reason).collect()
}

/// The ordering invariant for per-player output (plan §8 #6): players in id order (the
/// `BTreeMap`), and within one player's stream: own dispositions by ordinal, then AOI
/// despawns, AOI spawns, and moves, each group in entity-id order.
pub(crate) fn assert_output_order(t: &AppliedTick) {
    for out in t.outputs.values() {
        let rank = |o: &ObserverOutput| match o {
            ObserverOutput::Rejected(d) => (0, d.ordinal.0, None),
            ObserverOutput::Accepted { ordinal, .. } => (0, ordinal.0, None),
            ObserverOutput::Event(ZoneEvent::EntityDespawn { entity, .. }) => (1, 0, Some(*entity)),
            ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) => (2, 0, Some(*entity)),
            ObserverOutput::Event(ZoneEvent::EntityMove { entity, .. }) => (3, 0, Some(*entity)),
            ObserverOutput::Event(
                _e @ (ZoneEvent::AttackResult { .. }
                | ZoneEvent::EntityDied { .. }
                | ZoneEvent::EntityRespawned { .. }
                | ZoneEvent::StatsChanged { .. }
                | ZoneEvent::XpGained { .. }
                | ZoneEvent::LevelUp { .. }
                | ZoneEvent::TargetChanged { .. }
                | ZoneEvent::AttackStarted { .. }
                | ZoneEvent::AttackCancelled { .. }
                | ZoneEvent::HateChanged { .. }
                | ZoneEvent::NpcIntentionChanged { .. }),
            ) => (4, 0, None),
        };
        let mut ranks: Vec<_> = out.iter().map(rank).collect();
        // Facts keep their causal order: rank them by position.
        for (i, r) in ranks.iter_mut().enumerate() {
            if r.0 == 4 {
                r.1 = u64::try_from(i).unwrap();
            }
        }
        let mut sorted = ranks.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ranks, sorted, "output order is not canonical: {out:?}");
    }
}

#[test]
fn bounds_reject_min_above_max() {
    let a = Vec2Fixed::from_tiles(1, 0);
    let b = Vec2Fixed::from_tiles(0, 0);
    assert_eq!(ZoneBounds::new(a, b), Err(InvalidBounds));
    assert!(ZoneBounds::new(b, b).is_ok());
}

#[test]
fn spawn_player_emits_spawn_and_the_player_sees_itself() {
    let mut z = zone();
    let t = run(&mut z, vec![spawn_player(1, 10, 10)]);
    assert!(matches!(t.events.as_slice(), [ZoneEvent::EntitySpawn { .. }]));
    assert!(t.dispositions.is_empty());
    let own = &t.outputs[&id(1)];
    assert!(
        matches!(own.as_slice(), [ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. })] if *entity == id(1))
    );
    assert_eq!(t.server_time_ms, 1_000_000);
}

#[test]
fn spawn_outside_bounds_or_twice_is_rejected() {
    let mut z = zone();
    let t = run(
        &mut z,
        vec![
            spawn_player(1, -1, 0),
            spawn_player(2, 0, 0),
            spawn_player(2, 5, 5),
        ],
    );
    assert_eq!(reasons(&t), vec![RejectReason::OutOfBounds, RejectReason::AlreadyExists]);
    assert_eq!(t.dispositions[0].ordinal, Ordinal(0));
    assert_eq!(t.dispositions[1].ordinal, Ordinal(2));
    assert_eq!(z.entity_count(), 1);
    assert_eq!(z.next_ordinal(), Ordinal(3));
}

#[test]
fn npc_ids_come_from_the_seeded_rng() {
    let npc = || {
        ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "wolf".to_owned(),
            pos: Vec2Fixed::from_tiles(5, 5),
            speed: Speed::DEFAULT,
            combat: None,
        })
    };
    let ids = |epoch| {
        let mut z = ZoneState::new(
            ZoneSeed {
                zone: ZoneId(1),
                epoch,
            },
            zone().bounds(),
            0,
        );
        run(&mut z, vec![npc(), npc()])
            .events
            .iter()
            .map(ZoneEvent::entity)
            .collect::<Vec<_>>()
    };
    let a = ids(7);
    assert_eq!(a, ids(7));
    assert_ne!(a, ids(8));
    assert_ne!(a[0], a[1]);
    assert_eq!(a[0].as_uuid().get_version_num(), 4);
}

#[test]
fn move_to_an_unknown_entity_is_rejected() {
    let mut z = zone();
    let t = run(
        &mut z,
        vec![
            ZoneInput::system(ZoneCommand::MoveTo {
                entity: id(9),
                dest: Vec2Fixed::from_tiles(1, 1),
            }),
            ZoneInput::system(ZoneCommand::StopMove { entity: id(9) }),
            ZoneInput::system(ZoneCommand::ReplaceSession {
                entity: id(9),
                generation: GEN1,
            }),
        ],
    );
    assert_eq!(reasons(&t), vec![RejectReason::UnknownEntity; 3]);
}

#[test]
fn move_to_rejects_destinations_one_unit_outside_bounds() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0), spawn_player(2, 250, 250)]);
    let t = run(
        &mut z,
        vec![
            move_to(1, Vec2Fixed::new(Fixed::from_raw(-1), Fixed::ZERO)),
            move_to(1, Vec2Fixed::from_tiles(0, 0)),
            move_to(2, Vec2Fixed::from_tiles(256, 256)),
            move_to(2, Vec2Fixed::new(Fixed::from_tiles(256), Fixed::from_raw(256_001))),
        ],
    );
    assert_eq!(reasons(&t), vec![RejectReason::OutOfBounds, RejectReason::OutOfBounds]);
    assert_eq!(t.dispositions[0].ordinal, Ordinal(2));
    assert_eq!(t.dispositions[1].ordinal, Ordinal(5));
}

#[test]
fn move_to_rejects_one_unit_past_the_max_distance() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 100, 100)]);
    let over = Vec2Fixed::new(Fixed::from_raw(164_001), Fixed::from_tiles(100));
    let t = run(&mut z, vec![move_to(1, over)]);
    assert_eq!(reasons(&t), vec![RejectReason::TooFar]);
    let t = run(&mut z, vec![move_to(1, Vec2Fixed::from_tiles(164, 100))]);
    assert!(t.dispositions.is_empty());
}

#[test]
fn movement_emits_every_tick_and_arrival_once() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0)]);
    // 1 tile at 0.5 tiles/tick: two moves, the second is the arrival.
    let t = run(&mut z, vec![move_to(1, Vec2Fixed::from_tiles(1, 0))]);
    let first = ZoneEvent::EntityMove {
        tick: Tick(1),
        entity: id(1),
        pos: Vec2Fixed::new(Fixed::from_raw(500), Fixed::ZERO),
        dest: Some(Vec2Fixed::from_tiles(1, 0)),
        speed: Speed::DEFAULT,
    };
    assert_eq!(t.events, vec![first.clone()]);
    let ack = ObserverOutput::Accepted {
        ordinal: t.commands[0].ordinal,
        seq: t.commands[0].seq.unwrap(),
        tick: Tick(1),
    };
    assert_eq!(t.outputs[&id(1)], vec![ack, ObserverOutput::Event(first)]);
    let t = run(&mut z, Vec::new());
    assert_eq!(
        t.events,
        vec![ZoneEvent::EntityMove {
            tick: Tick(2),
            entity: id(1),
            pos: Vec2Fixed::from_tiles(1, 0),
            dest: None,
            speed: Speed::DEFAULT,
        }]
    );
    assert!(run(&mut z, Vec::new()).is_idle());
    assert_eq!(z.next_tick(), Tick(4));
}

#[test]
fn responses_come_first_in_ordinal_order_then_events() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 10, 10)]);
    let mv = |seq, x| {
        ZoneInput::session(
            id(1),
            GEN1,
            seq,
            ZoneCommand::MoveTo {
                entity: id(1),
                dest: Vec2Fixed::from_tiles(x, 10),
            },
        )
    };
    // Accepted, refused (too far), accepted: one response each, in the order sent.
    let t = run(&mut z, vec![mv(1, 12), mv(2, 200), mv(3, 11)]);
    let own = &t.outputs[&id(1)];
    let heads: Vec<(u32, bool)> = own
        .iter()
        .filter_map(|o| match o {
            ObserverOutput::Accepted { seq, tick, .. } => {
                assert_eq!(*tick, Tick(1));
                Some((*seq, true))
            },
            ObserverOutput::Rejected(d) => Some((d.seq.unwrap(), false)),
            ObserverOutput::Event(_) => None,
        })
        .collect();
    assert_eq!(heads, vec![(1, true), (2, false), (3, true)]);
    assert!(matches!(own.as_slice(), [_, _, _, ObserverOutput::Event(_)]));
    assert_output_order(&t);
}

#[test]
fn system_commands_and_unsequenced_session_commands_get_no_ack() {
    let mut z = zone();
    let t = run(&mut z, vec![spawn_player(1, 10, 10), spawn_player(2, 11, 11)]);
    assert!(t.outputs[&id(1)]
        .iter()
        .all(|o| matches!(o, ObserverOutput::Event(_))));
    let leave = ZoneInput {
        source: CommandSource::Session {
            entity: id(2),
            generation: GEN1,
        },
        seq: None,
        command: ZoneCommand::Despawn { entity: id(2) },
    };
    let t = run(&mut z, vec![leave]);
    assert!(t.dispositions.is_empty());
    assert!(t.outputs[&id(1)]
        .iter()
        .all(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntityDespawn { .. }))));
}

#[test]
fn stop_move_emits_a_stopped_move_only_when_moving() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0)]);
    let stop = || ZoneInput::session(id(1), GEN1, 1, ZoneCommand::StopMove { entity: id(1) });
    assert!(run(&mut z, vec![stop()]).events.is_empty());
    run(&mut z, vec![move_to(1, Vec2Fixed::from_tiles(10, 0))]);
    let t = run(&mut z, vec![stop()]);
    assert!(matches!(t.events.as_slice(), [ZoneEvent::EntityMove { dest: None, .. }]));
    assert!(run(&mut z, Vec::new()).events.is_empty());
}

#[test]
fn zero_speed_entities_accept_a_destination_but_emit_nothing() {
    let mut z = zone();
    run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: id(1),
            name: "rooted".to_owned(),
            pos: Vec2Fixed::from_tiles(0, 0),
            speed: Speed::from_milli_tiles_per_tick(0),
            generation: GEN1,
            load: None,
        })],
    );
    let t = run(&mut z, vec![move_to(1, Vec2Fixed::from_tiles(1, 0))]);
    assert!(t.dispositions.is_empty() && t.events.is_empty());
}

#[test]
fn despawn_removes_from_the_index_and_is_idempotent() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0)]);
    let despawn = || ZoneInput::system(ZoneCommand::Despawn { entity: id(1) });
    let t = run(&mut z, vec![despawn(), despawn()]);
    assert_eq!(
        t.events,
        vec![ZoneEvent::EntityDespawn {
            tick: Tick(1),
            entity: id(1)
        }]
    );
    assert!(t.dispositions.is_empty());
    assert_eq!(z.aoi().cell_count(), 0);
}

#[test]
fn sessions_may_only_steer_their_own_entity() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0), spawn_player(2, 1, 1)]);
    let as_1 = |cmd| ZoneInput::session(id(1), GEN1, 7, cmd);
    let t = run(
        &mut z,
        vec![
            as_1(ZoneCommand::MoveTo {
                entity: id(2),
                dest: Vec2Fixed::from_tiles(2, 2),
            }),
            as_1(ZoneCommand::Despawn { entity: id(2) }),
            as_1(ZoneCommand::ReplaceSession {
                entity: id(1),
                generation: SessionGeneration(9),
            }),
            as_1(ZoneCommand::SpawnNpc {
                name: "x".to_owned(),
                pos: Vec2Fixed::default(),
                speed: Speed::DEFAULT,
                combat: None,
            }),
        ],
    );
    assert_eq!(reasons(&t), vec![RejectReason::NotPermitted; 4]);
    // Rejections reach the issuing session first, with its seq, and never anyone else.
    let own = &t.outputs[&id(1)];
    assert_eq!(own.len(), 4);
    assert!(own
        .iter()
        .all(|o| matches!(o, ObserverOutput::Rejected(d) if d.seq == Some(7))));
    assert!(!t.outputs.contains_key(&id(2)));
}

#[test]
fn replacement_fences_the_old_session_and_resends_the_aoi() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0), spawn_player(2, 1, 1)]);
    let gen2 = SessionGeneration(2);
    let t = run(
        &mut z,
        vec![
            ZoneInput::system(ZoneCommand::ReplaceSession {
                entity: id(1),
                generation: gen2,
            }),
            ZoneInput::system(ZoneCommand::ReplaceSession {
                entity: id(1),
                generation: gen2,
            }),
        ],
    );
    assert_eq!(reasons(&t), vec![RejectReason::StaleSession]);
    // The new session is told about everything in its AOI again, in id order.
    let spawned: Vec<EntityId> = t.outputs[&id(1)]
        .iter()
        .map(|o| match o {
            ObserverOutput::Event(e) => e.entity(),
            ObserverOutput::Rejected(_) | ObserverOutput::Accepted { .. } => NIL,
        })
        .collect();
    assert_eq!(spawned, vec![id(1), id(2)]);
    assert!(!t.outputs.contains_key(&id(2)));

    // The old socket's disconnect cannot remove the replacement.
    let stale = ZoneInput::session(id(1), GEN1, 3, ZoneCommand::Despawn { entity: id(1) });
    let t = run(&mut z, vec![stale]);
    assert_eq!(reasons(&t), vec![RejectReason::StaleSession]);
    assert!(z.entity(id(1)).is_some());
    // The stale session's rejection is not routed to the current session.
    assert!(t.outputs.is_empty());

    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: id(1),
            generation: SessionGeneration(3),
        })],
    );
    assert!(t.dispositions.is_empty());
    let npc = ZoneInput::system(ZoneCommand::SpawnNpc {
        name: "n".to_owned(),
        pos: Vec2Fixed::default(),
        speed: Speed::DEFAULT,
        combat: None,
    });
    let t = run(&mut z, vec![npc]);
    let npc_id = t.events[0].entity();
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: npc_id,
            generation: gen2,
        })],
    );
    assert_eq!(reasons(&t), vec![RejectReason::NotAPlayer]);
}

const NIL: EntityId = EntityId::from_uuid(Uuid::nil());

#[test]
fn aoi_diffs_spawn_and_despawn_as_entities_cross_cells() {
    let mut z = zone();
    // Observer in cell (0,0); walker in cell (3,0), outside the 3x3 block.
    let t = run(&mut z, vec![spawn_player(1, 10, 10), spawn_player(2, 100, 10)]);
    assert_output_order(&t);
    assert_eq!(t.outputs[&id(1)].len(), 1, "observer sees only itself");

    // Walker heads west at 0.5 tiles/tick; the observer's block ends at x = 64 (cell 1).
    let walk = ZoneInput::session(
        id(2),
        GEN1,
        1,
        ZoneCommand::MoveTo {
            entity: id(2),
            dest: Vec2Fixed::from_tiles(40, 10),
        },
    );
    let mut t = run(&mut z, vec![walk]);
    let mut ticks = 1;
    while t.outputs.get(&id(1)).is_none_or(Vec::is_empty) {
        t = run(&mut z, Vec::new());
        ticks += 1;
    }
    assert_eq!(ticks, 73, "x = 100 - 0.5n first drops below 64 at n = 73");
    let ObserverOutput::Event(ZoneEvent::EntitySpawn {
        entity,
        dest,
        speed,
        ..
    }) = &t.outputs[&id(1)][0]
    else {
        unreachable!("expected a spawn, got {:?}", t.outputs[&id(1)]);
    };
    assert_eq!(
        (*entity, *dest, *speed),
        (id(2), Some(Vec2Fixed::from_tiles(40, 10)), Speed::DEFAULT)
    );

    // Next tick it is known, so the observer gets a move, not another spawn.
    let t = run(&mut z, Vec::new());
    assert!(matches!(
        t.outputs[&id(1)].as_slice(),
        [ObserverOutput::Event(ZoneEvent::EntityMove { .. })]
    ));

    // Walk it back out of range: the observer gets a despawn.
    let back = ZoneInput::system(ZoneCommand::MoveTo {
        entity: id(2),
        dest: Vec2Fixed::from_tiles(120, 10),
    });
    let mut t = run(&mut z, vec![back]);
    for _ in 0..10 {
        if t.outputs.get(&id(1)).is_some_and(|o| {
            matches!(o.as_slice(), [ObserverOutput::Event(ZoneEvent::EntityDespawn { .. })])
        }) {
            return;
        }
        assert_output_order(&t);
        t = run(&mut z, Vec::new());
    }
    unreachable!("walker never left the observer's AOI");
}

#[test]
fn observers_crossing_cells_get_spawns_for_what_they_now_see() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 10, 10), spawn_player(2, 100, 10)]);
    let walk = ZoneInput::session(
        id(1),
        GEN1,
        1,
        ZoneCommand::MoveTo {
            entity: id(1),
            dest: Vec2Fixed::from_tiles(70, 10),
        },
    );
    let mut t = run(&mut z, vec![walk]);
    let mut guard = 0;
    while !t.outputs[&id(1)]
        .iter()
        .any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) if *entity == id(2)))
    {
        assert_output_order(&t);
        t = run(&mut z, Vec::new());
        guard += 1;
        assert!(guard < 200, "observer never saw entity 2");
    }
    assert_output_order(&t);
    // Entity 2 sees the observer arrive in its own AOI on the same tick.
    assert!(t.outputs[&id(2)]
        .iter()
        .any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) if *entity == id(1))));
}

#[test]
fn drafts_that_do_not_continue_the_zone_are_refused() {
    let mut z = zone();
    let mut draft = z.draft(vec![spawn_player(1, 0, 0)]);
    draft.tick = Tick(5);
    assert_eq!(
        z.clone().run_tick(draft.clone()),
        Err(TickError::Tick {
            expected: Tick(0),
            got: Tick(5)
        })
    );
    draft.tick = Tick(0);
    draft.commands[0].ordinal = Ordinal(3);
    assert_eq!(
        z.clone().run_tick(draft.clone()),
        Err(TickError::Ordinal {
            expected: Ordinal(0),
            got: Ordinal(3)
        })
    );
    draft.commands[0].ordinal = Ordinal(0);
    draft.epoch = 1;
    assert_eq!(
        z.clone().run_tick(draft.clone()),
        Err(TickError::Epoch {
            expected: 7,
            got: 1
        })
    );
    draft.epoch = 7;
    assert!(z.run_tick(draft).is_ok());
}

#[test]
fn rng_depends_on_zone_and_epoch_and_survives_a_snapshot() {
    let mut a = zone();
    let mut other_epoch = ZoneState::new(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 8,
        },
        a.bounds(),
        0,
    );
    let rolls_a: Vec<u16> = (0..16).map(|_| a.roll_permille()).collect();
    let rolls_b: Vec<u16> = (0..16).map(|_| other_epoch.roll_permille()).collect();
    assert_ne!(rolls_a, rolls_b);
    assert!(rolls_a.iter().all(|r| *r < 1000));

    run(
        &mut a,
        vec![
            spawn_player(1, 3, 3),
            move_to(1, Vec2Fixed::from_tiles(30, 30)),
        ],
    );
    let json = serde_json::to_string(&a.snapshot()).unwrap();
    let mut restored = ZoneState::from_snapshot(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(restored, a);
    assert_eq!(restored.roll_permille(), a.roll_permille());
    let next = vec![move_to(1, Vec2Fixed::from_tiles(3, 30))];
    assert_eq!(run(&mut restored, next.clone()), run(&mut a, next));
}

#[test]
fn invalid_snapshots_are_refused() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 0, 0)]);
    let check = |edit: &dyn Fn(&mut ZoneSnapshot), want: SnapshotError| {
        let mut snap = z.snapshot();
        edit(&mut snap);
        assert_eq!(ZoneState::from_snapshot(snap), Err(want));
    };
    check(
        &|s| s.entities.push(s.entities[0].clone()),
        SnapshotError::DuplicateEntity(id(1)),
    );
    check(
        &|s| s.entities[0].pos = Vec2Fixed::from_tiles(-1, 0),
        SnapshotError::OutOfBounds(id(1)),
    );
    check(
        &|s| s.entities[0].pos = Vec2Fixed::from_tiles(100, 0),
        SnapshotError::AoiMismatch,
    );
    check(&|s| s.rng.key[0] ^= 1, SnapshotError::SeedMismatch);
    check(&|s| s.meta.schema_version = 99, SnapshotError::Schema(99));
}

#[test]
fn seed_key_layout_is_fixed() {
    let key = ZoneSeed {
        zone: ZoneId(0x0102_0304),
        epoch: 5,
    }
    .key();
    assert_eq!(&key[..20], b"nightfall.zone.rng.1");
    assert_eq!(&key[20..24], &[4, 3, 2, 1]);
    assert_eq!(&key[24..], &[5, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn targeting_validates_before_mutation_and_repeats_are_silent() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10),
            spawn_player(2, 10, 10),
            ZoneInput::system(ZoneCommand::SpawnNpc {
                name: "combat fixture".into(),
                pos: Vec2Fixed::from_tiles(11, 10),
                speed: Speed::DEFAULT,
                combat: None,
            }),
        ],
    );
    let npc = z
        .entities
        .values()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap()
        .id;
    let select = |target| {
        ZoneInput::session(
            id(1),
            GEN1,
            1,
            ZoneCommand::SetTarget {
                entity: id(1),
                target,
            },
        )
    };
    for (target, reason) in [
        (id(999), RejectReason::UnknownEntity),
        (id(2), RejectReason::NonAttackableTarget),
        (npc, RejectReason::NonAttackableTarget),
    ] {
        let before = z.snapshot();
        let t = run(&mut z, vec![select(Some(target))]);
        assert_eq!(reasons(&t), vec![reason]);
        assert!(t.events.is_empty());
        assert_eq!(before.entities, z.snapshot().entities);
        assert_eq!(before.rng, z.snapshot().rng);
    }
    z.entities.get_mut(&npc).unwrap().targeting.attackable = true;
    let t = run(&mut z, vec![select(Some(npc)), select(Some(npc))]);
    assert!(t.dispositions.is_empty());
    assert_eq!(
        t.events,
        vec![ZoneEvent::TargetChanged {
            tick: t.tick,
            entity: id(1),
            target: Some(npc)
        }]
    );
    let out = &t.outputs[&id(1)];
    assert!(matches!(
        out.as_slice(),
        [
            ObserverOutput::Accepted { .. },
            ObserverOutput::Accepted { .. },
            ObserverOutput::Event(ZoneEvent::TargetChanged { .. })
        ]
    ));
    assert!(!t.outputs.contains_key(&id(2)), "selection is owner-only");
    let restored = ZoneState::from_snapshot(z.snapshot()).unwrap();
    assert_eq!(restored.entity(id(1)).unwrap().targeting.target, Some(npc));
    let t = run(&mut z, vec![select(None), select(None)]);
    assert_eq!(t.events.len(), 1);
    assert_eq!(z.entity(id(1)).unwrap().targeting.target, None);
}

#[test]
fn combat_commands_are_fenced_and_stubs_do_not_change_state_or_rng() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 10, 10), spawn_player(2, 10, 10)]);
    for command in [
        ZoneCommand::SetTarget {
            entity: id(1),
            target: None,
        },
        ZoneCommand::Attack { entity: id(1) },
        ZoneCommand::StopAttack { entity: id(1) },
        ZoneCommand::Respawn { entity: id(1) },
    ] {
        for (actor, generation, reason) in [
            (id(2), GEN1, RejectReason::NotPermitted),
            (id(1), SessionGeneration(0), RejectReason::StaleSession),
        ] {
            let before = z.snapshot();
            let t = run(&mut z, vec![ZoneInput::session(actor, generation, 1, command.clone())]);
            assert_eq!(reasons(&t), vec![reason]);
            assert_eq!(before.entities, z.snapshot().entities);
            assert_eq!(before.rng, z.snapshot().rng);
            assert!(t.events.is_empty());
        }
    }
    // Respawn is still a stub (E2.4); a noncombat player (no rules, no load) cannot attack.
    for (command, reason) in [
        (ZoneCommand::Attack { entity: id(1) }, RejectReason::NotPermitted),
        (ZoneCommand::StopAttack { entity: id(1) }, RejectReason::NotPermitted),
        (ZoneCommand::Respawn { entity: id(1) }, RejectReason::NotYetImplemented),
    ] {
        let before = z.snapshot();
        let t = run(
            &mut z,
            vec![
                ZoneInput::session(id(1), GEN1, 2, command.clone()),
                ZoneInput::session(id(1), GEN1, 3, command),
            ],
        );
        assert_eq!(reasons(&t), vec![reason; 2]);
        assert_eq!(t.commands.len(), 2);
        assert_eq!(before.entities, z.snapshot().entities);
        assert_eq!(before.rng, z.snapshot().rng);
        assert!(t.events.is_empty());
    }
}

#[test]
fn combat_validation_rejects_dead_or_missing_actor_and_dead_or_distant_target() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10),
            ZoneInput::system(ZoneCommand::SpawnNpc {
                name: "target".into(),
                pos: Vec2Fixed::from_tiles(11, 10),
                speed: Speed::DEFAULT,
                combat: None,
            }),
        ],
    );
    let npc = z
        .entities
        .values()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap()
        .id;
    z.entities.get_mut(&npc).unwrap().targeting.attackable = true;
    for command in [
        ZoneCommand::SetTarget {
            entity: id(1),
            target: Some(npc),
        },
        ZoneCommand::Attack { entity: id(1) },
        ZoneCommand::StopAttack { entity: id(1) },
    ] {
        z.entities.get_mut(&id(1)).unwrap().targeting.dead = true;
        let before = z.snapshot();
        let t = run(&mut z, vec![ZoneInput::session(id(1), GEN1, 1, command)]);
        assert_eq!(reasons(&t), vec![RejectReason::DeadActor]);
        assert_eq!(before.entities, z.snapshot().entities);
        assert_eq!(before.rng, z.snapshot().rng);
        assert!(t.events.is_empty());
    }
    let t = run(
        &mut z,
        vec![ZoneInput::session(
            id(1),
            GEN1,
            2,
            ZoneCommand::Respawn { entity: id(1) },
        )],
    );
    assert_eq!(reasons(&t), vec![RejectReason::NotYetImplemented]);
    assert!(z.entity(id(1)).unwrap().targeting.dead);
    for command in [
        ZoneCommand::SetTarget {
            entity: id(999),
            target: None,
        },
        ZoneCommand::Attack { entity: id(999) },
        ZoneCommand::StopAttack { entity: id(999) },
        ZoneCommand::Respawn { entity: id(999) },
    ] {
        let t = run(&mut z, vec![ZoneInput::session(id(999), GEN1, 3, command)]);
        assert_eq!(reasons(&t), vec![RejectReason::UnknownEntity]);
        assert!(t.events.is_empty());
    }
    z.entities.get_mut(&id(1)).unwrap().targeting.dead = false;
    z.entities.get_mut(&npc).unwrap().targeting.dead = true;
    let select = ZoneInput::session(
        id(1),
        GEN1,
        4,
        ZoneCommand::SetTarget {
            entity: id(1),
            target: Some(npc),
        },
    );
    let before = z.snapshot();
    let t = run(&mut z, vec![select.clone()]);
    assert_eq!(reasons(&t), vec![RejectReason::NonAttackableTarget]);
    assert_eq!(before.entities, z.snapshot().entities);
    assert_eq!(before.rng, z.snapshot().rng);
    assert!(t.events.is_empty());
    let e = z.entities.get_mut(&npc).unwrap();
    z.aoi.remove(npc, e.pos);
    e.pos = Vec2Fixed::from_tiles(200, 10);
    e.targeting.dead = false;
    z.aoi.insert(npc, e.pos);
    let before = z.snapshot();
    let t = run(&mut z, vec![select]);
    assert_eq!(reasons(&t), vec![RejectReason::TargetNotInAoi]);
    assert_eq!(before.entities, z.snapshot().entities);
    assert_eq!(before.rng, z.snapshot().rng);
    assert!(t.events.is_empty());
}
