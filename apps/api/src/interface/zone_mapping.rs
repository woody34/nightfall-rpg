//! `nightfall.v1` world.proto <-> `domain::zone` conversions. Pure functions.
//!
//! This is the only place zone values become floats: the wire carries tile units as `f32`
//! (`Position`) and speed as tiles per second, while the zone is integer fixed-point (plan
//! D7). Inbound floats are rounded to the nearest 1/1000 tile once, at the edge, so everything
//! downstream (including replay of the recorded inbound bytes) is integer and deterministic.

use bytes::Bytes;
use prost::Message as _;
use thiserror::Error;

use super::grpc::pb;
use pb::world_event::Event;

use crate::application::session::PlayerSpawn;
use crate::domain::{Character, Race};

use crate::domain::zone::{
    AttackOutcome, CombatView, Disposition, EntityId, EntityKind, Fixed, ObserverOutput,
    PlayerLoad, RejectReason, SessionGeneration, Speed, Swing, SwingCancel, Tick, Vec2Fixed,
    ZoneCommand, ZoneEvent, TICK_MS, UNITS_PER_TILE,
};

/// An inbound message that cannot become a zone command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MappingError {
    /// The `oneof intent` was empty.
    #[error("intent is required")]
    MissingIntent,
    /// Target is neither empty nor a UUID.
    #[error("target must be a UUID or empty")]
    InvalidTarget,
    /// `MoveTo` without a destination.
    #[error("destination is required")]
    MissingDestination,
    /// NaN, infinite, or beyond the fixed-point range.
    #[error("coordinate is not a finite tile position")]
    InvalidCoordinate,
}

impl MappingError {
    /// The `IntentRejected` reason world.proto specifies: a non-finite or out-of-range
    /// coordinate is `OUT_OF_BOUNDS`, a malformed message is `INVALID`.
    #[must_use]
    pub const fn reject_reason(self) -> pb::RejectReason {
        match self {
            Self::InvalidCoordinate => pb::RejectReason::OutOfBounds,
            Self::InvalidTarget | Self::MissingIntent | Self::MissingDestination => {
                pb::RejectReason::Invalid
            },
        }
    }
}

/// Fixed-point to wire tiles. `i32 -> f64` is exact and the single division is correctly
/// rounded, so the same `Fixed` always yields the same `f32` bits.
#[must_use]
pub fn fixed_to_tiles(v: Fixed) -> f32 {
    // Narrowing f64 -> f32 is the wire format's precision; positions within +/-16k tiles keep
    // full 1/1000 tile resolution.
    #[allow(clippy::cast_possible_truncation)]
    let tiles = (f64::from(v.raw()) / f64::from(UNITS_PER_TILE)) as f32;
    tiles
}

/// Wire tiles to fixed-point, rounding half away from zero to the nearest 1/1000 tile.
pub fn tiles_to_fixed(tiles: f32) -> Result<Fixed, MappingError> {
    let units = (f64::from(tiles) * f64::from(UNITS_PER_TILE)).round();
    if !units.is_finite() || units < f64::from(i32::MIN) || units > f64::from(i32::MAX) {
        return Err(MappingError::InvalidCoordinate);
    }
    // In range and integral after the checks above, so the cast is exact.
    #[allow(clippy::cast_possible_truncation)]
    let raw = units as i32;
    Ok(Fixed::from_raw(raw))
}

/// Domain position to wire position.
#[must_use]
pub fn position_to_pb(p: Vec2Fixed) -> pb::Position {
    pb::Position {
        x: fixed_to_tiles(p.x),
        y: fixed_to_tiles(p.y),
    }
}

/// Wire position to domain position.
pub fn position_from_pb(p: pb::Position) -> Result<Vec2Fixed, MappingError> {
    Ok(Vec2Fixed::new(tiles_to_fixed(p.x)?, tiles_to_fixed(p.y)?))
}

/// Speed in tiles per second for `EntityMove.speed`.
#[must_use]
pub fn speed_to_tiles_per_second(s: Speed) -> f32 {
    let ticks_per_second = 1000 / TICK_MS;
    // u32 * 10 fits f64 exactly.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let v = (f64::from(s.milli_tiles_per_tick()) * ticks_per_second as f64
        / f64::from(UNITS_PER_TILE)) as f32;
    v
}

/// What a loaded character spawns as: its id as the entity id (plan §8 #6), its saved
/// position rounded to the nearest milli-tile, default speed, and its race's starting
/// fighter class at its level. The session registry replaces this provisional load with
/// committed progression and position under the lifecycle lock before queueing admission.
pub fn player_spawn(c: &Character) -> Result<PlayerSpawn, MappingError> {
    Ok(PlayerSpawn {
        entity: EntityId::from_uuid(c.id.as_uuid()),
        name: c.name.as_str().to_owned(),
        pos: Vec2Fixed::new(tiles_to_fixed(c.position.x)?, tiles_to_fixed(c.position.y)?),
        speed: Speed::DEFAULT,
        load: Some(Box::new(PlayerLoad {
            level: c.level,
            xp: c.xp,
            ..PlayerLoad::fresh(
                crate::domain::character_progression::base_class_profile(
                    c.class_state.base_class_id,
                )
                .unwrap_or(starter_class(c.race)),
            )
        })),
    })
}

/// The class data id (`packages/data/classes/<id>.toml`) a race starts as. Phase 1 has one
/// fixed starter weapon, so every character starts as its race's fighter.
#[must_use]
pub const fn starter_class(race: Race) -> &'static str {
    match race {
        Race::Human => "human_fighter",
        Race::Elf => "elven_fighter",
        Race::DarkElf => "dark_fighter",
        Race::Orc => "orc_fighter",
        Race::Dwarf => "dwarven_fighter",
    }
}

/// Domain entity kind to the wire enum value.
#[must_use]
pub fn kind_to_pb(k: EntityKind) -> pb::EntityKind {
    match k {
        EntityKind::Player => pb::EntityKind::Player,
        EntityKind::Npc => pb::EntityKind::Npc,
    }
}

/// The command a session's inbound intent asks for, on behalf of the session's own entity.
pub fn command_from_pb(
    entity: EntityId,
    intent: Option<&pb::client_message::Intent>,
) -> Result<ZoneCommand, MappingError> {
    match intent.ok_or(MappingError::MissingIntent)? {
        pb::client_message::Intent::MoveTo(req) => {
            let dest = req
                .destination
                .as_ref()
                .ok_or(MappingError::MissingDestination)?;
            Ok(ZoneCommand::MoveTo {
                entity,
                dest: position_from_pb(*dest)?,
            })
        },
        pb::client_message::Intent::StopMove(_) => Ok(ZoneCommand::StopMove { entity }),
        pb::client_message::Intent::SetTarget(req) => Ok(ZoneCommand::SetTarget {
            entity,
            target: if req.entity_id.is_empty() {
                None
            } else {
                Some(EntityId::from_uuid(
                    uuid::Uuid::parse_str(&req.entity_id)
                        .map_err(|_| MappingError::InvalidTarget)?,
                ))
            },
        }),
        pb::client_message::Intent::Attack(_) => Ok(ZoneCommand::Attack { entity }),
        pb::client_message::Intent::StopAttack(_) => Ok(ZoneCommand::StopAttack { entity }),
        pb::client_message::Intent::Respawn(_) => Ok(ZoneCommand::Respawn { entity }),
    }
}

/// `EntitySpawn`. Its message has no movement fields, so a spawn of a moving entity is
/// followed by an `EntityMove` (see [`world_event_to_pb`]).
#[must_use]
pub fn spawn_to_pb(
    entity: EntityId,
    kind: EntityKind,
    name: &str,
    pos: Vec2Fixed,
    generation: SessionGeneration,
    combat: Option<&CombatView>,
) -> pb::WorldEvent {
    let mut spawn = pb::EntitySpawn {
        entity_id: entity.to_string(),
        name: name.to_owned(),
        position: Some(position_to_pb(pos)),
        kind: kind_to_pb(kind).into(),
        // Generations count admissions of one account; u32 is ample and saturates.
        session_generation: u32::try_from(generation.0).unwrap_or(u32::MAX),
        ..Default::default()
    };
    if let Some(c) = combat {
        spawn.combatant = true;
        spawn.template_id = c.template.clone().unwrap_or_default();
        spawn.life_incarnation = c.incarnation;
        spawn.dead = c.dead;
        spawn.attackable = c.attackable;
        spawn.hp = c.hp;
        spawn.max_hp = c.max_hp;
        spawn.level = c.level;
        spawn.pending_swing = c.swing.map(|s| swing_to_pb(entity, &s));
    }
    world(Event::Spawn(spawn))
}

/// A swing in flight as `AttackStarted`.
#[must_use]
pub fn swing_to_pb(attacker: EntityId, s: &Swing) -> pb::AttackStarted {
    pb::AttackStarted {
        attacker: attacker.to_string(),
        target: s.target.to_string(),
        tick: s.start.0,
        impact_tick: s.impact.0,
        ready_tick: s.ready.0,
        target_incarnation: s.target_incarnation,
    }
}

/// Domain cancel reason to the wire enum.
#[must_use]
pub const fn cancel_to_pb(r: SwingCancel) -> pb::SwingCancelReason {
    match r {
        SwingCancel::Stopped => pb::SwingCancelReason::Stopped,
        SwingCancel::TargetChanged => pb::SwingCancelReason::TargetChanged,
        SwingCancel::Moved => pb::SwingCancelReason::Moved,
        SwingCancel::OutOfRange => pb::SwingCancelReason::OutOfRange,
        SwingCancel::TargetLost => pb::SwingCancelReason::TargetLost,
        SwingCancel::AttackerDied => pb::SwingCancelReason::AttackerDied,
    }
}

/// `EntityMove`. `server_time_ms` is `AppliedTick::server_time_ms`, never a clock read. A
/// stopped or arrived entity (`dest: None`) is sent with destination and speed zero, as
/// world.proto specifies.
#[must_use]
pub fn move_to_pb(
    entity: EntityId,
    pos: Vec2Fixed,
    dest: Option<Vec2Fixed>,
    speed: Speed,
    tick: Tick,
    server_time_ms: i64,
) -> pb::WorldEvent {
    let (destination, speed) = match dest {
        Some(d) => (position_to_pb(d), speed_to_tiles_per_second(speed)),
        None => (pb::Position { x: 0.0, y: 0.0 }, 0.0),
    };
    world(Event::Move(pb::EntityMove {
        entity_id: entity.to_string(),
        position: Some(position_to_pb(pos)),
        destination: Some(destination),
        speed,
        server_time_ms,
        tick: tick.0,
    }))
}

/// `EntityDespawn`.
#[must_use]
pub fn despawn_to_pb(entity: EntityId) -> pb::WorldEvent {
    world(Event::Despawn(pb::EntityDespawn {
        entity_id: entity.to_string(),
    }))
}

fn world(event: Event) -> pb::WorldEvent {
    pb::WorldEvent { event: Some(event) }
}

/// The wire messages for one zone event, in order. `server_time_ms` is the event's tick's
/// (`AppliedTick::server_time_ms`).
#[must_use]
#[allow(clippy::too_many_lines)] // one arm per event, mirroring world.proto
pub fn world_event_to_pb(ev: &ZoneEvent, server_time_ms: i64) -> Vec<pb::WorldEvent> {
    match ev {
        ZoneEvent::EntitySpawn {
            identity,
            tick,
            entity,
            kind,
            name,
            pos,
            dest,
            speed,
            generation,
            combat,
        } => {
            let mut spawn = spawn_to_pb(*entity, *kind, name, *pos, *generation, combat.as_ref());
            if let (Some(identity), Some(Event::Spawn(s))) = (identity, &mut spawn.event) {
                s.race = match identity.race {
                    Race::Human => pb::Race::Human,
                    Race::Elf => pb::Race::Elf,
                    Race::DarkElf => pb::Race::DarkElf,
                    Race::Orc => pb::Race::Orc,
                    Race::Dwarf => pb::Race::Dwarf,
                }
                .into();
                s.class_id = identity.class_id.0;
                s.sex = match identity.appearance.sex {
                    crate::domain::subclass::Sex::Male => pb::Sex::Male,
                    crate::domain::subclass::Sex::Female => pb::Sex::Female,
                }
                .into();
                s.hair_style = identity.appearance.hair_style;
                s.hair_color = identity.appearance.hair_color;
                s.face = identity.appearance.face;
                s.state_tick = tick.0;
            }
            match dest {
                Some(_) => vec![
                    spawn,
                    move_to_pb(*entity, *pos, *dest, *speed, *tick, server_time_ms),
                ],
                None => vec![spawn],
            }
        },
        ZoneEvent::EntityMove {
            tick,
            entity,
            pos,
            dest,
            speed,
        } => vec![move_to_pb(
            *entity,
            *pos,
            *dest,
            *speed,
            *tick,
            server_time_ms,
        )],
        ZoneEvent::AttackResult {
            attacker,
            target,
            tick,
            outcome,
            damage,
            target_hp_after,
            target_incarnation,
        } => vec![world(Event::AttackResult(pb::AttackResult {
            attacker: attacker.to_string(),
            target: target.to_string(),
            tick: tick.0,
            outcome: match outcome {
                AttackOutcome::Miss => pb::AttackOutcome::Miss,
                AttackOutcome::Hit => pb::AttackOutcome::Hit,
                AttackOutcome::Crit => pb::AttackOutcome::Crit,
            }
            .into(),
            damage: *damage,
            target_hp_after: *target_hp_after,
            target_incarnation: *target_incarnation,
        }))],
        ZoneEvent::EntityDied {
            entity,
            tick,
            killer,
            incarnation,
        } => vec![world(Event::EntityDied(pb::EntityDied {
            entity: entity.to_string(),
            tick: tick.0,
            killer: killer.map(|id| id.to_string()).unwrap_or_default(),
            incarnation: *incarnation,
        }))],
        ZoneEvent::AttackStarted {
            tick,
            attacker,
            target,
            target_incarnation,
            impact,
            ready,
        } => vec![world(Event::AttackStarted(swing_to_pb(
            *attacker,
            &Swing {
                target: *target,
                target_incarnation: *target_incarnation,
                start: *tick,
                impact: *impact,
                ready: *ready,
            },
        )))],
        ZoneEvent::AttackCancelled {
            tick,
            attacker,
            target,
            reason,
        } => vec![world(Event::AttackCancelled(pb::AttackCancelled {
            attacker: attacker.to_string(),
            target: target.to_string(),
            tick: tick.0,
            reason: cancel_to_pb(*reason).into(),
        }))],
        ZoneEvent::ClassChanged {
            tick,
            entity,
            class_id,
            generation,
        } => vec![world(Event::ClassChanged(pb::ClassChanged {
            entity: entity.to_string(),
            class_id: class_id.0,
            tick: tick.0,
            session_generation: u32::try_from(generation.0).unwrap_or(u32::MAX),
        }))],
        // Internal; never in an observer's output.
        ZoneEvent::ClassTransfer { .. }
        | ZoneEvent::HateChanged { .. }
        | ZoneEvent::NpcIntentionChanged { .. }
        | ZoneEvent::Progression(_) => Vec::new(),
        ZoneEvent::EntityRespawned {
            entity,
            tick,
            position,
            hp,
            incarnation,
        } => vec![world(Event::EntityRespawned(pb::EntityRespawned {
            entity: entity.to_string(),
            tick: tick.0,
            position: Some(position_to_pb(*position)),
            hp: *hp,
            incarnation: *incarnation,
        }))],
        ZoneEvent::StatsChanged {
            class,
            tick,
            entity,
            hp,
            max_hp,
            mp,
            max_mp,
            level,
            xp,
            ..
        } => vec![world(Event::StatsChanged(pb::StatsChanged {
            entity: entity.to_string(),
            hp: *hp,
            max_hp: *max_hp,
            mp: *mp,
            max_mp: *max_mp,
            level: *level,
            xp: *xp,
            cp: class.as_ref().map_or(0, |c| c.cp),
            max_cp: class.as_ref().map_or(0, |c| c.max_cp),
            class_id: class.as_ref().map_or(0, |c| c.class_id.0),
            sp: class.as_ref().map_or(0, |c| c.sp),
            token_tier_1_count: class.as_ref().map_or(0, |c| c.token_tier_1_count),
            token_tier_2_count: class.as_ref().map_or(0, |c| c.token_tier_2_count),
            tick: if class.is_some() { tick.0 } else { 0 },
        }))],
        ZoneEvent::XpGained {
            entity,
            amount,
            total,
            ..
        } => vec![world(Event::XpGained(pb::XpGained {
            entity: entity.to_string(),
            amount: *amount,
            total: *total,
        }))],
        ZoneEvent::LevelUp { entity, level, .. } => vec![world(Event::LevelUp(pb::LevelUp {
            entity: entity.to_string(),
            level: *level,
        }))],
        ZoneEvent::TargetChanged { entity, target, .. } => {
            vec![world(Event::TargetChanged(pb::TargetChanged {
                entity: entity.to_string(),
                target: target.map(|id| id.to_string()).unwrap_or_default(),
            }))]
        },
        ZoneEvent::EntityDespawn { entity, .. } => vec![despawn_to_pb(*entity)],
    }
}

/// Domain rejection reason to the wire enum. Reasons without their own wire value become
/// `INVALID`; [`RejectReason::detail`] says which. `OVERLOADED` and `RATE_LIMITED` are decided
/// by the session before a command reaches the zone, so they never come from here.
#[must_use]
pub fn reject_reason_to_pb(r: RejectReason) -> pb::RejectReason {
    match r {
        RejectReason::DeadActor => pb::RejectReason::DeadActor,
        RejectReason::NonAttackableTarget => pb::RejectReason::NonAttackableTarget,
        RejectReason::TargetNotInAoi => pb::RejectReason::TargetNotInAoi,
        RejectReason::OutOfRange => pb::RejectReason::OutOfRange,
        RejectReason::Protected => pb::RejectReason::Protected,
        RejectReason::NotYetImplemented => pb::RejectReason::NotYetImplemented,
        RejectReason::OutOfBounds => pb::RejectReason::OutOfBounds,
        RejectReason::TooFar => pb::RejectReason::TooFar,
        RejectReason::UnknownEntity => pb::RejectReason::UnknownEntity,
        RejectReason::TransferConflict
        | RejectReason::TransferIneligible
        | RejectReason::TransferRequirement
        | RejectReason::InCombat
        | RejectReason::ClassMasterTooFar
        | RejectReason::AlreadyExists
        | RejectReason::NotPermitted
        | RejectReason::StaleSession
        | RejectReason::NotAPlayer
        | RejectReason::InvalidLoad
        | RejectReason::NotDead => pb::RejectReason::Invalid,
    }
}

/// `IntentRejected` for a disposition. `None` for system commands, which have no client to
/// tell.
#[must_use]
pub fn disposition_to_pb(d: &Disposition) -> Option<pb::IntentRejected> {
    Some(pb::IntentRejected {
        seq: d.seq?,
        reason: reject_reason_to_pb(d.reason).into(),
        detail: d.reason.detail().to_owned(),
    })
}

/// One item of a player's output stream as server messages, in order.
#[must_use]
pub fn observer_output_to_pb(o: &ObserverOutput, server_time_ms: i64) -> Vec<pb::ServerMessage> {
    use pb::server_message::Payload;
    match o {
        ObserverOutput::Event(ev) => world_event_to_pb(ev, server_time_ms)
            .into_iter()
            .map(|w| pb::ServerMessage {
                payload: Some(Payload::Event(w)),
            })
            .collect(),
        ObserverOutput::Accepted { seq, tick, .. } => vec![pb::ServerMessage {
            payload: Some(Payload::Ack(pb::Ack {
                seq: *seq,
                tick: tick.0,
            })),
        }],
        ObserverOutput::Rejected(d) => disposition_to_pb(d)
            .map(|r| pb::ServerMessage {
                payload: Some(Payload::Rejected(r)),
            })
            .into_iter()
            .collect(),
    }
}

/// A player's whole output for one tick as encoded `ServerMessage` frames, in order: one
/// binary WebSocket frame each. The session sends exactly these bytes (plan §8 #6) and replay
/// compares them.
#[must_use]
pub fn encode_observer_outputs(outputs: &[ObserverOutput], server_time_ms: i64) -> Vec<Bytes> {
    outputs
        .iter()
        .flat_map(|o| observer_output_to_pb(o, server_time_ms))
        .map(|m| Bytes::from(m.encode_to_vec()))
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::panic
)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::zone::{CommandSource, Ordinal};

    #[test]
    fn tiles_round_trip_through_fixed() {
        for raw in [0, 1, -1, 999, 1000, 123_456, -123_456, 16_000_000] {
            let f = Fixed::from_raw(raw);
            assert_eq!(tiles_to_fixed(fixed_to_tiles(f)).unwrap(), f, "raw {raw}");
        }
    }

    #[test]
    fn inbound_floats_round_to_the_nearest_thousandth() {
        assert_eq!(tiles_to_fixed(1.0004).unwrap(), Fixed::from_raw(1000));
        assert_eq!(tiles_to_fixed(1.0006).unwrap(), Fixed::from_raw(1001));
        assert_eq!(tiles_to_fixed(-2.5).unwrap(), Fixed::from_raw(-2500));
    }

    #[test]
    fn non_finite_or_huge_coordinates_are_rejected() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 3.0e6, -3.0e6] {
            assert_eq!(tiles_to_fixed(bad), Err(MappingError::InvalidCoordinate), "{bad}");
            assert_eq!(
                MappingError::InvalidCoordinate.reject_reason(),
                pb::RejectReason::OutOfBounds
            );
        }
    }

    #[test]
    fn default_speed_is_five_tiles_per_second() {
        assert_eq!(speed_to_tiles_per_second(Speed::DEFAULT), 5.0);
    }

    #[test]
    fn move_intent_becomes_move_to_and_requires_a_destination() {
        let e = EntityId::from_uuid(Uuid::from_u128(1));
        let intent = pb::client_message::Intent::MoveTo(pb::MoveToRequest {
            destination: Some(pb::Position { x: 3.5, y: -1.25 }),
        });
        assert_eq!(
            command_from_pb(e, Some(&intent)),
            Ok(ZoneCommand::MoveTo {
                entity: e,
                dest: Vec2Fixed::new(Fixed::from_raw(3500), Fixed::from_raw(-1250)),
            })
        );
        let empty = pb::client_message::Intent::MoveTo(pb::MoveToRequest { destination: None });
        assert_eq!(command_from_pb(e, Some(&empty)), Err(MappingError::MissingDestination));
        assert_eq!(command_from_pb(e, None), Err(MappingError::MissingIntent));
        let stop = pb::client_message::Intent::StopMove(pb::StopMoveRequest {});
        assert_eq!(command_from_pb(e, Some(&stop)), Ok(ZoneCommand::StopMove { entity: e }));
    }

    #[test]
    fn stopped_move_has_a_zero_destination_and_the_given_time() {
        let ev = ZoneEvent::EntityMove {
            tick: Tick(12),
            entity: EntityId::from_uuid(Uuid::from_u128(1)),
            pos: Vec2Fixed::from_tiles(2, 3),
            dest: None,
            speed: Speed::DEFAULT,
        };
        let wire = world_event_to_pb(&ev, 2_200);
        let [pb::WorldEvent {
            event: Some(Event::Move(m)),
        }] = wire.as_slice()
        else {
            panic!("expected one move, got {wire:?}");
        };
        assert_eq!((m.server_time_ms, m.tick), (2_200, 12));
        assert_eq!(m.speed, 0.0, "stopped entities are sent with speed zero");
        assert_eq!(m.destination, Some(pb::Position { x: 0.0, y: 0.0 }));
        assert_eq!(m.position, Some(pb::Position { x: 2.0, y: 3.0 }));
    }

    #[test]
    fn spawn_of_a_moving_entity_is_followed_by_its_move() {
        let ev = ZoneEvent::EntitySpawn {
            identity: None,
            tick: Tick(1),
            entity: EntityId::from_uuid(Uuid::from_u128(1)),
            kind: EntityKind::Npc,
            name: "wolf".to_owned(),
            pos: Vec2Fixed::from_tiles(1, 1),
            dest: Some(Vec2Fixed::from_tiles(4, 5)),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(3),
            combat: None,
        };
        let wire = world_event_to_pb(&ev, 0);
        assert!(matches!(
            wire.as_slice(),
            [
                pb::WorldEvent {
                    event: Some(Event::Spawn(_))
                },
                pb::WorldEvent {
                    event: Some(Event::Move(_))
                }
            ]
        ));
    }

    #[test]
    fn accepted_commands_become_acks_with_the_applied_tick() {
        let out = ObserverOutput::Accepted {
            ordinal: Ordinal(3),
            seq: 41,
            tick: Tick(9),
        };
        assert_eq!(
            observer_output_to_pb(&out, 0),
            vec![pb::ServerMessage {
                payload: Some(pb::server_message::Payload::Ack(pb::Ack { seq: 41, tick: 9 })),
            }]
        );
        let frames = encode_observer_outputs(&[out], 0);
        let decoded = pb::ServerMessage::decode(frames[0].as_ref()).unwrap();
        assert!(
            matches!(decoded.payload, Some(pb::server_message::Payload::Ack(a)) if a.seq == 41)
        );
    }

    #[test]
    fn dispositions_map_to_intent_rejected() {
        let d = Disposition {
            ordinal: Ordinal(4),
            source: CommandSource::System,
            seq: Some(9),
            tick_seen: Tick(1),
            reason: RejectReason::StaleSession,
        };
        assert_eq!(
            disposition_to_pb(&d),
            Some(pb::IntentRejected {
                seq: 9,
                reason: pb::RejectReason::Invalid.into(),
                detail: "session generation is not current".to_owned(),
            })
        );
        let too_far = Disposition {
            reason: RejectReason::TooFar,
            ..d
        };
        let msgs = observer_output_to_pb(&ObserverOutput::Rejected(too_far), 0);
        assert!(matches!(
            msgs.as_slice(),
            [pb::ServerMessage { payload: Some(pb::server_message::Payload::Rejected(r)) }]
                if r.reason == i32::from(pb::RejectReason::TooFar)
        ));
        assert_eq!(disposition_to_pb(&Disposition { seq: None, ..d }), None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod combat_tests {
    use super::*;
    use crate::domain::zone::AttackOutcome;
    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn attack_result_all_fields_round_trip() {
        let event = ZoneEvent::AttackResult {
            attacker: id(1),
            target: id(2),
            tick: Tick(31),
            outcome: AttackOutcome::Crit,
            damage: 25,
            target_hp_after: 26,
            target_incarnation: 4,
        };
        let expected = pb::WorldEvent {
            event: Some(Event::AttackResult(pb::AttackResult {
                attacker: id(1).to_string(),
                target: id(2).to_string(),
                tick: 31,
                outcome: pb::AttackOutcome::Crit.into(),
                damage: 25,
                target_hp_after: 26,
                target_incarnation: 4,
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn entity_died_all_fields_round_trip() {
        let event = ZoneEvent::EntityDied {
            entity: id(1),
            tick: Tick(31),
            killer: Some(id(3)),
            incarnation: 2,
        };
        let expected = pb::WorldEvent {
            event: Some(Event::EntityDied(pb::EntityDied {
                entity: id(1).to_string(),
                tick: 31,
                killer: id(3).to_string(),
                incarnation: 2,
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn entity_respawned_all_fields_round_trip() {
        let event = ZoneEvent::EntityRespawned {
            entity: id(1),
            tick: Tick(31),
            position: Vec2Fixed::from_tiles(23, 24),
            hp: 24,
            incarnation: 3,
        };
        let expected = pb::WorldEvent {
            event: Some(Event::EntityRespawned(pb::EntityRespawned {
                entity: id(1).to_string(),
                tick: 31,
                position: Some(pb::Position { x: 23.0, y: 24.0 }),
                hp: 24,
                incarnation: 3,
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn stats_changed_all_fields_round_trip() {
        let event = ZoneEvent::StatsChanged {
            class: None,
            tick: Tick(31),
            entity: id(1),
            hp: 22,
            max_hp: 23,
            mp: 24,
            max_mp: 25,
            level: 26,
            xp: u64::MAX,
        };
        let expected = pb::WorldEvent {
            event: Some(Event::StatsChanged(pb::StatsChanged {
                entity: id(1).to_string(),
                hp: 22,
                max_hp: 23,
                mp: 24,
                max_mp: 25,
                level: 26,
                xp: u64::MAX,
                ..Default::default()
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn xp_gained_all_fields_round_trip() {
        let event = ZoneEvent::XpGained {
            tick: Tick(31),
            entity: id(1),
            amount: u64::MAX,
            total: u64::MAX,
        };
        let expected = pb::WorldEvent {
            event: Some(Event::XpGained(pb::XpGained {
                entity: id(1).to_string(),
                amount: u64::MAX,
                total: u64::MAX,
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn level_up_all_fields_round_trip() {
        let event = ZoneEvent::LevelUp {
            tick: Tick(31),
            entity: id(1),
            level: 22,
        };
        let expected = pb::WorldEvent {
            event: Some(Event::LevelUp(pb::LevelUp {
                entity: id(1).to_string(),
                level: 22,
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn target_changed_all_fields_round_trip() {
        let event = ZoneEvent::TargetChanged {
            tick: Tick(31),
            entity: id(1),
            target: Some(id(2)),
        };
        let expected = pb::WorldEvent {
            event: Some(Event::TargetChanged(pb::TargetChanged {
                entity: id(1).to_string(),
                target: id(2).to_string(),
            })),
        };
        assert_eq!(world_event_to_pb(&event, 3100), vec![expected.clone()]);
        let frame = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Event(expected)),
        };
        assert_eq!(pb::ServerMessage::decode(frame.encode_to_vec().as_slice()).unwrap(), frame);
    }

    #[test]
    fn new_intents_round_trip_and_always_use_session_actor() {
        use pb::client_message::Intent;
        for (intent, expected) in [
            (
                Intent::SetTarget(pb::SetTargetRequest {
                    entity_id: id(2).to_string(),
                }),
                ZoneCommand::SetTarget {
                    entity: id(1),
                    target: Some(id(2)),
                },
            ),
            (
                Intent::SetTarget(pb::SetTargetRequest {
                    entity_id: String::new(),
                }),
                ZoneCommand::SetTarget {
                    entity: id(1),
                    target: None,
                },
            ),
            (Intent::Attack(pb::AttackRequest {}), ZoneCommand::Attack { entity: id(1) }),
            (
                Intent::StopAttack(pb::StopAttackRequest {}),
                ZoneCommand::StopAttack { entity: id(1) },
            ),
            (Intent::Respawn(pb::RespawnRequest {}), ZoneCommand::Respawn { entity: id(1) }),
        ] {
            let original = pb::ClientMessage {
                seq: 123,
                intent: Some(intent),
            };
            let decoded = pb::ClientMessage::decode(original.encode_to_vec().as_slice()).unwrap();
            assert_eq!(original, decoded);
            assert_eq!(command_from_pb(id(1), decoded.intent.as_ref()).unwrap(), expected);
        }
        assert_eq!(
            command_from_pb(
                id(1),
                Some(&Intent::SetTarget(pb::SetTargetRequest {
                    entity_id: "bad-id".into()
                }))
            ),
            Err(MappingError::InvalidTarget)
        );
    }

    #[test]
    fn all_attack_outcomes_and_new_reasons_round_trip() {
        for outcome in [
            pb::AttackOutcome::Miss,
            pb::AttackOutcome::Hit,
            pb::AttackOutcome::Crit,
        ] {
            let event = pb::AttackResult {
                attacker: id(1).to_string(),
                target: id(2).to_string(),
                tick: 23,
                outcome: outcome.into(),
                damage: 0,
                target_hp_after: 100,
                target_incarnation: 1,
            };
            assert_eq!(pb::AttackResult::decode(event.encode_to_vec().as_slice()).unwrap(), event);
        }
        for (domain, wire) in [
            (RejectReason::DeadActor, pb::RejectReason::DeadActor),
            (RejectReason::NonAttackableTarget, pb::RejectReason::NonAttackableTarget),
            (RejectReason::TargetNotInAoi, pb::RejectReason::TargetNotInAoi),
            (RejectReason::OutOfRange, pb::RejectReason::OutOfRange),
            (RejectReason::Protected, pb::RejectReason::Protected),
            (RejectReason::NotYetImplemented, pb::RejectReason::NotYetImplemented),
        ] {
            assert_eq!(reject_reason_to_pb(domain), wire);
            let event = pb::IntentRejected {
                seq: 25,
                reason: wire.into(),
                detail: domain.detail().into(),
            };
            assert_eq!(
                pb::IntentRejected::decode(event.encode_to_vec().as_slice()).unwrap(),
                event
            );
        }
    }
}
