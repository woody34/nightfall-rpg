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
use crate::domain::Character;

use crate::domain::zone::{
    Disposition, EntityId, EntityKind, Fixed, ObserverOutput, RejectReason, SessionGeneration,
    Speed, Tick, Vec2Fixed, ZoneCommand, ZoneEvent, TICK_MS, UNITS_PER_TILE,
};

/// An inbound message that cannot become a zone command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MappingError {
    /// The `oneof intent` was empty.
    #[error("intent is required")]
    MissingIntent,
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
            Self::MissingIntent | Self::MissingDestination => pb::RejectReason::Invalid,
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
/// position rounded to the nearest milli-tile, default speed.
pub fn player_spawn(c: &Character) -> Result<PlayerSpawn, MappingError> {
    Ok(PlayerSpawn {
        entity: EntityId::from_uuid(c.id.as_uuid()),
        name: c.name.as_str().to_owned(),
        pos: Vec2Fixed::new(tiles_to_fixed(c.position.x)?, tiles_to_fixed(c.position.y)?),
        speed: Speed::DEFAULT,
    })
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
) -> pb::WorldEvent {
    world(Event::Spawn(pb::EntitySpawn {
        entity_id: entity.to_string(),
        name: name.to_owned(),
        position: Some(position_to_pb(pos)),
        kind: kind_to_pb(kind).into(),
        // Generations count admissions of one account; u32 is ample and saturates.
        session_generation: u32::try_from(generation.0).unwrap_or(u32::MAX),
    }))
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
pub fn world_event_to_pb(ev: &ZoneEvent, server_time_ms: i64) -> Vec<pb::WorldEvent> {
    match ev {
        ZoneEvent::EntitySpawn {
            tick,
            entity,
            kind,
            name,
            pos,
            dest,
            speed,
            generation,
        } => {
            let spawn = spawn_to_pb(*entity, *kind, name, *pos, *generation);
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
        ZoneEvent::EntityDespawn { entity, .. } => vec![despawn_to_pb(*entity)],
    }
}

/// Domain rejection reason to the wire enum. Reasons without their own wire value become
/// `INVALID`; [`RejectReason::detail`] says which. `OVERLOADED` and `RATE_LIMITED` are decided
/// by the session before a command reaches the zone, so they never come from here.
#[must_use]
pub fn reject_reason_to_pb(r: RejectReason) -> pb::RejectReason {
    match r {
        RejectReason::OutOfBounds => pb::RejectReason::OutOfBounds,
        RejectReason::TooFar => pb::RejectReason::TooFar,
        RejectReason::UnknownEntity => pb::RejectReason::UnknownEntity,
        RejectReason::AlreadyExists
        | RejectReason::NotPermitted
        | RejectReason::StaleSession
        | RejectReason::NotAPlayer => pb::RejectReason::Invalid,
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
            tick: Tick(1),
            entity: EntityId::from_uuid(Uuid::from_u128(1)),
            kind: EntityKind::Npc,
            name: "wolf".to_owned(),
            pos: Vec2Fixed::from_tiles(1, 1),
            dest: Some(Vec2Fixed::from_tiles(4, 5)),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(3),
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
