//! Protobuf encoding of the replay log's per-tick and per-frame records, as hand-derived
//! `prost` messages so the schema lives next to the conversion code and needs no build step.
//! The equivalent `.proto` (package `nightfall.replay.v1`):
//!
//! ```text
//! message AppliedTickRecord {
//!   uint32 zone = 1; uint64 epoch = 2; uint64 tick = 3; int64 server_time_ms = 4;
//!   repeated Command commands = 5; repeated Disposition dispositions = 6;
//!   repeated PlayerOutput outputs = 7;
//!   OutputForm output_form = 8;
//! }
//! enum OutputForm { ENCODED = 0; SHA256 = 1; }  // SHA256: outputs too large for one message
//! message Session { bytes entity = 1; uint64 generation = 2; }   // absent = System source
//! message Vec2 { sint32 x = 1; sint32 y = 2; }                     // milli-tiles
//! message Command {
//!   uint64 ordinal = 1; Session session = 2; optional uint32 seq = 3;
//!   oneof kind { SpawnPlayer spawn_player = 4; SpawnNpc spawn_npc = 5; EntityRef despawn = 6;
//!                Replace replace_session = 7; MoveTo move_to = 8; EntityRef stop_move = 9; }
//! }
//! message Disposition { uint64 ordinal = 1; Session session = 2; optional uint32 seq = 3;
//!                       uint64 tick_seen = 4; Reason reason = 5; }
//! message PlayerOutput { bytes entity = 1;
//!                        bytes encoded = 2;   // Outputs; set when output_form = ENCODED
//!                        bytes sha256 = 3; }  // SHA-256 of the Outputs bytes; when SHA256
//! message Outputs { repeated Output items = 1; }
//! message Output { oneof item { Spawn spawn = 1; Move move = 2; Despawn despawn = 3;
//!                               Disposition rejected = 4; } }
//! message SessionIn  { bytes session = 1; uint64 seq = 2; uint32 zone = 3; uint64 epoch = 4;
//!                      uint64 tick_seen = 5; int64 recv_unix_ms = 6; bytes frame = 7; }
//! message SessionOut { bytes session = 1; uint32 zone = 2; uint64 epoch = 3; uint64 tick = 4;
//!                      bytes frame = 5; }
//! ```
//!
//! Tag numbers and enum values are part of the log format: never renumber, only add.

use bytes::Bytes;
use prost::Message;
use uuid::Uuid;

use super::record::{
    AppliedTickRecord, CodecError, OutputForm, PlayerOutput, SessionInRecord, SessionOutRecord,
};
use crate::domain::zone::{
    AppliedCommand, CommandSource, Disposition, EntityId, EntityKind, Fixed, ObserverOutput,
    Ordinal, RejectReason, SessionGeneration, Speed, Tick, Vec2Fixed, ZoneCommand, ZoneEvent,
    ZoneId,
};

#[derive(Clone, PartialEq, Message)]
struct PbRecord {
    #[prost(uint32, tag = "1")]
    zone: u32,
    #[prost(uint64, tag = "2")]
    epoch: u64,
    #[prost(uint64, tag = "3")]
    tick: u64,
    #[prost(int64, tag = "4")]
    server_time_ms: i64,
    #[prost(message, repeated, tag = "5")]
    commands: Vec<PbCommand>,
    #[prost(message, repeated, tag = "6")]
    dispositions: Vec<PbDisposition>,
    #[prost(message, repeated, tag = "7")]
    outputs: Vec<PbPlayerOutput>,
    #[prost(int32, tag = "8")]
    output_form: i32,
}

#[derive(Clone, PartialEq, Message)]
struct PbSession {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    generation: u64,
}

#[derive(Clone, Copy, PartialEq, Message)]
struct PbVec {
    #[prost(sint32, tag = "1")]
    x: i32,
    #[prost(sint32, tag = "2")]
    y: i32,
}

#[derive(Clone, PartialEq, Message)]
struct PbCommand {
    #[prost(uint64, tag = "1")]
    ordinal: u64,
    #[prost(message, optional, tag = "2")]
    session: Option<PbSession>,
    #[prost(uint32, optional, tag = "3")]
    seq: Option<u32>,
    #[prost(oneof = "PbCommandKind", tags = "4, 5, 6, 7, 8, 9")]
    kind: Option<PbCommandKind>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum PbCommandKind {
    #[prost(message, tag = "4")]
    SpawnPlayer(PbSpawnPlayer),
    #[prost(message, tag = "5")]
    SpawnNpc(PbSpawnNpc),
    #[prost(message, tag = "6")]
    Despawn(PbEntityRef),
    #[prost(message, tag = "7")]
    ReplaceSession(PbReplace),
    #[prost(message, tag = "8")]
    MoveTo(PbMoveTo),
    #[prost(message, tag = "9")]
    StopMove(PbEntityRef),
}

#[derive(Clone, PartialEq, Message)]
struct PbSpawnPlayer {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(message, optional, tag = "3")]
    pos: Option<PbVec>,
    #[prost(uint32, tag = "4")]
    speed: u32,
    #[prost(uint64, tag = "5")]
    generation: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbSpawnNpc {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, optional, tag = "2")]
    pos: Option<PbVec>,
    #[prost(uint32, tag = "3")]
    speed: u32,
}

#[derive(Clone, PartialEq, Message)]
struct PbEntityRef {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct PbReplace {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    generation: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbMoveTo {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    dest: Option<PbVec>,
}

#[derive(Clone, PartialEq, Message)]
struct PbDisposition {
    #[prost(uint64, tag = "1")]
    ordinal: u64,
    #[prost(message, optional, tag = "2")]
    session: Option<PbSession>,
    #[prost(uint32, optional, tag = "3")]
    seq: Option<u32>,
    #[prost(uint64, tag = "4")]
    tick_seen: u64,
    #[prost(int32, tag = "5")]
    reason: i32,
}

#[derive(Clone, PartialEq, Message)]
struct PbPlayerOutput {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(bytes = "bytes", tag = "2")]
    encoded: Bytes,
    #[prost(bytes = "bytes", tag = "3")]
    sha256: Bytes,
}

#[derive(Clone, PartialEq, Message)]
struct PbOutputs {
    #[prost(message, repeated, tag = "1")]
    items: Vec<PbOutput>,
}

#[derive(Clone, PartialEq, Message)]
struct PbOutput {
    #[prost(oneof = "PbOutputItem", tags = "1, 2, 3, 4")]
    item: Option<PbOutputItem>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum PbOutputItem {
    #[prost(message, tag = "1")]
    Spawn(PbSpawnEvent),
    #[prost(message, tag = "2")]
    Move(PbMoveEvent),
    #[prost(message, tag = "3")]
    Despawn(PbDespawnEvent),
    #[prost(message, tag = "4")]
    Rejected(PbDisposition),
}

#[derive(Clone, PartialEq, Message)]
struct PbSpawnEvent {
    #[prost(uint64, tag = "1")]
    tick: u64,
    #[prost(bytes = "vec", tag = "2")]
    entity: Vec<u8>,
    #[prost(int32, tag = "3")]
    kind: i32,
    #[prost(string, tag = "4")]
    name: String,
    #[prost(message, optional, tag = "5")]
    pos: Option<PbVec>,
    #[prost(message, optional, tag = "6")]
    dest: Option<PbVec>,
    #[prost(uint32, tag = "7")]
    speed: u32,
    #[prost(uint64, tag = "8")]
    generation: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbMoveEvent {
    #[prost(uint64, tag = "1")]
    tick: u64,
    #[prost(bytes = "vec", tag = "2")]
    entity: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    pos: Option<PbVec>,
    #[prost(message, optional, tag = "4")]
    dest: Option<PbVec>,
    #[prost(uint32, tag = "5")]
    speed: u32,
}

#[derive(Clone, PartialEq, Message)]
struct PbDespawnEvent {
    #[prost(uint64, tag = "1")]
    tick: u64,
    #[prost(bytes = "vec", tag = "2")]
    entity: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct PbSessionIn {
    #[prost(bytes = "vec", tag = "1")]
    session: Vec<u8>,
    #[prost(uint64, tag = "2")]
    seq: u64,
    #[prost(uint32, tag = "3")]
    zone: u32,
    #[prost(uint64, tag = "4")]
    epoch: u64,
    #[prost(uint64, tag = "5")]
    tick_seen: u64,
    #[prost(int64, tag = "6")]
    recv_unix_ms: i64,
    #[prost(bytes = "bytes", tag = "7")]
    frame: Bytes,
}

#[derive(Clone, PartialEq, Message)]
struct PbSessionOut {
    #[prost(bytes = "vec", tag = "1")]
    session: Vec<u8>,
    #[prost(uint32, tag = "2")]
    zone: u32,
    #[prost(uint64, tag = "3")]
    epoch: u64,
    #[prost(uint64, tag = "4")]
    tick: u64,
    #[prost(bytes = "bytes", tag = "5")]
    frame: Bytes,
}

// ---- encoding -------------------------------------------------------------------------------

pub(super) fn encode_record(r: &AppliedTickRecord) -> Vec<u8> {
    record_to_pb(r).encode_to_vec()
}

pub(super) fn record_encoded_len(r: &AppliedTickRecord) -> usize {
    record_to_pb(r).encoded_len()
}

/// Wire values of [`OutputForm`].
const fn output_form_to_pb(f: OutputForm) -> i32 {
    match f {
        OutputForm::Encoded => 0,
        OutputForm::Sha256 => 1,
    }
}

fn record_to_pb(r: &AppliedTickRecord) -> PbRecord {
    let output = |o: &PlayerOutput| {
        let (encoded, sha256) = match r.output_form {
            OutputForm::Encoded => (o.bytes.clone(), Bytes::new()),
            OutputForm::Sha256 => (Bytes::new(), o.bytes.clone()),
        };
        PbPlayerOutput {
            entity: entity_bytes(o.entity),
            encoded,
            sha256,
        }
    };
    PbRecord {
        zone: r.zone.0,
        epoch: r.epoch,
        tick: r.tick.0,
        server_time_ms: r.server_time_ms,
        commands: r.commands.iter().map(command_to_pb).collect(),
        dispositions: r.dispositions.iter().map(disposition_to_pb).collect(),
        outputs: r.outputs.iter().map(output).collect(),
        output_form: output_form_to_pb(r.output_form),
    }
}

pub(super) fn encode_outputs(items: &[ObserverOutput]) -> Vec<u8> {
    PbOutputs {
        items: items.iter().map(output_to_pb).collect(),
    }
    .encode_to_vec()
}

pub(super) fn encode_session_in(r: &SessionInRecord) -> Vec<u8> {
    PbSessionIn {
        session: r.session.as_bytes().to_vec(),
        seq: r.seq,
        zone: r.zone.0,
        epoch: r.epoch,
        tick_seen: r.tick_seen.0,
        recv_unix_ms: r.recv_unix_ms,
        frame: r.frame.clone(),
    }
    .encode_to_vec()
}

pub(super) fn encode_session_out(r: &SessionOutRecord) -> Vec<u8> {
    PbSessionOut {
        session: r.session.as_bytes().to_vec(),
        zone: r.zone.0,
        epoch: r.epoch,
        tick: r.tick.0,
        frame: r.frame.clone(),
    }
    .encode_to_vec()
}

fn entity_bytes(id: EntityId) -> Vec<u8> {
    id.as_uuid().as_bytes().to_vec()
}

fn vec_to_pb(v: Vec2Fixed) -> PbVec {
    PbVec {
        x: v.x.raw(),
        y: v.y.raw(),
    }
}

fn source_to_pb(source: CommandSource) -> Option<PbSession> {
    match source {
        CommandSource::System => None,
        CommandSource::Session { entity, generation } => Some(PbSession {
            entity: entity_bytes(entity),
            generation: generation.0,
        }),
    }
}

fn command_to_pb(c: &AppliedCommand) -> PbCommand {
    let kind = match &c.command {
        ZoneCommand::SpawnPlayer {
            entity,
            name,
            pos,
            speed,
            generation,
        } => PbCommandKind::SpawnPlayer(PbSpawnPlayer {
            entity: entity_bytes(*entity),
            name: name.clone(),
            pos: Some(vec_to_pb(*pos)),
            speed: speed.milli_tiles_per_tick(),
            generation: generation.0,
        }),
        ZoneCommand::SpawnNpc { name, pos, speed } => PbCommandKind::SpawnNpc(PbSpawnNpc {
            name: name.clone(),
            pos: Some(vec_to_pb(*pos)),
            speed: speed.milli_tiles_per_tick(),
        }),
        ZoneCommand::Despawn { entity } => PbCommandKind::Despawn(PbEntityRef {
            entity: entity_bytes(*entity),
        }),
        ZoneCommand::ReplaceSession { entity, generation } => {
            PbCommandKind::ReplaceSession(PbReplace {
                entity: entity_bytes(*entity),
                generation: generation.0,
            })
        },
        ZoneCommand::MoveTo { entity, dest } => PbCommandKind::MoveTo(PbMoveTo {
            entity: entity_bytes(*entity),
            dest: Some(vec_to_pb(*dest)),
        }),
        ZoneCommand::StopMove { entity } => PbCommandKind::StopMove(PbEntityRef {
            entity: entity_bytes(*entity),
        }),
    };
    PbCommand {
        ordinal: c.ordinal.0,
        session: source_to_pb(c.source),
        seq: c.seq,
        kind: Some(kind),
    }
}

/// Wire values of [`RejectReason`]. Zero is reserved for "unset".
const fn reason_to_pb(r: RejectReason) -> i32 {
    match r {
        RejectReason::UnknownEntity => 1,
        RejectReason::OutOfBounds => 2,
        RejectReason::TooFar => 3,
        RejectReason::AlreadyExists => 4,
        RejectReason::NotPermitted => 5,
        RejectReason::StaleSession => 6,
        RejectReason::NotAPlayer => 7,
    }
}

fn reason_from_pb(v: i32) -> Result<RejectReason, CodecError> {
    Ok(match v {
        1 => RejectReason::UnknownEntity,
        2 => RejectReason::OutOfBounds,
        3 => RejectReason::TooFar,
        4 => RejectReason::AlreadyExists,
        5 => RejectReason::NotPermitted,
        6 => RejectReason::StaleSession,
        7 => RejectReason::NotAPlayer,
        other => return Err(CodecError(format!("unknown reject reason {other}"))),
    })
}

fn disposition_to_pb(d: &Disposition) -> PbDisposition {
    PbDisposition {
        ordinal: d.ordinal.0,
        session: source_to_pb(d.source),
        seq: d.seq,
        tick_seen: d.tick_seen.0,
        reason: reason_to_pb(d.reason),
    }
}

const fn kind_to_pb(k: EntityKind) -> i32 {
    match k {
        EntityKind::Player => 1,
        EntityKind::Npc => 2,
    }
}

fn output_to_pb(o: &ObserverOutput) -> PbOutput {
    let item = match o {
        ObserverOutput::Rejected(d) => PbOutputItem::Rejected(disposition_to_pb(d)),
        ObserverOutput::Event(ZoneEvent::EntitySpawn {
            tick,
            entity,
            kind,
            name,
            pos,
            dest,
            speed,
            generation,
        }) => PbOutputItem::Spawn(PbSpawnEvent {
            tick: tick.0,
            entity: entity_bytes(*entity),
            kind: kind_to_pb(*kind),
            name: name.clone(),
            pos: Some(vec_to_pb(*pos)),
            dest: dest.map(vec_to_pb),
            speed: speed.milli_tiles_per_tick(),
            generation: generation.0,
        }),
        ObserverOutput::Event(ZoneEvent::EntityMove {
            tick,
            entity,
            pos,
            dest,
            speed,
        }) => PbOutputItem::Move(PbMoveEvent {
            tick: tick.0,
            entity: entity_bytes(*entity),
            pos: Some(vec_to_pb(*pos)),
            dest: dest.map(vec_to_pb),
            speed: speed.milli_tiles_per_tick(),
        }),
        ObserverOutput::Event(ZoneEvent::EntityDespawn { tick, entity }) => {
            PbOutputItem::Despawn(PbDespawnEvent {
                tick: tick.0,
                entity: entity_bytes(*entity),
            })
        },
    };
    PbOutput { item: Some(item) }
}

// ---- decoding -------------------------------------------------------------------------------

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError(e.to_string())
}

fn output_form_from_pb(v: i32) -> Result<OutputForm, CodecError> {
    match v {
        0 => Ok(OutputForm::Encoded),
        1 => Ok(OutputForm::Sha256),
        other => Err(CodecError(format!("unknown output form {other}"))),
    }
}

fn player_output_from_pb(o: PbPlayerOutput, form: OutputForm) -> Result<PlayerOutput, CodecError> {
    let fits = match form {
        OutputForm::Encoded => o.sha256.is_empty(),
        OutputForm::Sha256 => o.encoded.is_empty() && o.sha256.len() == 32,
    };
    if !fits {
        return Err(CodecError(format!(
            "player output does not match form {form:?} ({} encoded bytes, {} digest bytes)",
            o.encoded.len(),
            o.sha256.len()
        )));
    }
    let bytes = match form {
        OutputForm::Encoded => o.encoded,
        OutputForm::Sha256 => o.sha256,
    };
    Ok(PlayerOutput {
        entity: entity_from(&o.entity)?,
        bytes,
    })
}

pub(super) fn decode_record(bytes: &[u8]) -> Result<AppliedTickRecord, CodecError> {
    let pb = PbRecord::decode(bytes).map_err(err)?;
    let output_form = output_form_from_pb(pb.output_form)?;
    Ok(AppliedTickRecord {
        zone: ZoneId(pb.zone),
        epoch: pb.epoch,
        tick: Tick(pb.tick),
        server_time_ms: pb.server_time_ms,
        commands: pb
            .commands
            .into_iter()
            .map(command_from_pb)
            .collect::<Result<_, _>>()?,
        dispositions: pb
            .dispositions
            .into_iter()
            .map(disposition_from_pb)
            .collect::<Result<_, _>>()?,
        outputs: pb
            .outputs
            .into_iter()
            .map(|o| player_output_from_pb(o, output_form))
            .collect::<Result<_, _>>()?,
        output_form,
    })
}

pub(super) fn decode_outputs(bytes: &[u8]) -> Result<Vec<ObserverOutput>, CodecError> {
    PbOutputs::decode(bytes)
        .map_err(err)?
        .items
        .into_iter()
        .map(output_from_pb)
        .collect()
}

pub(super) fn decode_session_in(bytes: &[u8]) -> Result<SessionInRecord, CodecError> {
    let pb = PbSessionIn::decode(bytes).map_err(err)?;
    Ok(SessionInRecord {
        session: uuid_from(&pb.session)?,
        seq: pb.seq,
        zone: ZoneId(pb.zone),
        epoch: pb.epoch,
        tick_seen: Tick(pb.tick_seen),
        recv_unix_ms: pb.recv_unix_ms,
        frame: pb.frame,
    })
}

pub(super) fn decode_session_out(bytes: &[u8]) -> Result<SessionOutRecord, CodecError> {
    let pb = PbSessionOut::decode(bytes).map_err(err)?;
    Ok(SessionOutRecord {
        session: uuid_from(&pb.session)?,
        zone: ZoneId(pb.zone),
        epoch: pb.epoch,
        tick: Tick(pb.tick),
        frame: pb.frame,
    })
}

fn uuid_from(bytes: &[u8]) -> Result<Uuid, CodecError> {
    Uuid::from_slice(bytes).map_err(err)
}

fn entity_from(bytes: &[u8]) -> Result<EntityId, CodecError> {
    uuid_from(bytes).map(EntityId::from_uuid)
}

fn vec_from(v: Option<PbVec>) -> Result<Vec2Fixed, CodecError> {
    let v = v.ok_or_else(|| CodecError("missing position".to_owned()))?;
    Ok(Vec2Fixed::new(Fixed::from_raw(v.x), Fixed::from_raw(v.y)))
}

fn source_from(s: Option<PbSession>) -> Result<CommandSource, CodecError> {
    Ok(match s {
        None => CommandSource::System,
        Some(s) => CommandSource::Session {
            entity: entity_from(&s.entity)?,
            generation: SessionGeneration(s.generation),
        },
    })
}

fn command_from_pb(c: PbCommand) -> Result<AppliedCommand, CodecError> {
    let command = match c.kind {
        None => return Err(CodecError("command without a kind".to_owned())),
        Some(PbCommandKind::SpawnPlayer(s)) => ZoneCommand::SpawnPlayer {
            entity: entity_from(&s.entity)?,
            name: s.name,
            pos: vec_from(s.pos)?,
            speed: Speed::from_milli_tiles_per_tick(s.speed),
            generation: SessionGeneration(s.generation),
        },
        Some(PbCommandKind::SpawnNpc(s)) => ZoneCommand::SpawnNpc {
            name: s.name,
            pos: vec_from(s.pos)?,
            speed: Speed::from_milli_tiles_per_tick(s.speed),
        },
        Some(PbCommandKind::Despawn(r)) => ZoneCommand::Despawn {
            entity: entity_from(&r.entity)?,
        },
        Some(PbCommandKind::ReplaceSession(r)) => ZoneCommand::ReplaceSession {
            entity: entity_from(&r.entity)?,
            generation: SessionGeneration(r.generation),
        },
        Some(PbCommandKind::MoveTo(m)) => ZoneCommand::MoveTo {
            entity: entity_from(&m.entity)?,
            dest: vec_from(m.dest)?,
        },
        Some(PbCommandKind::StopMove(r)) => ZoneCommand::StopMove {
            entity: entity_from(&r.entity)?,
        },
    };
    Ok(AppliedCommand {
        ordinal: Ordinal(c.ordinal),
        source: source_from(c.session)?,
        seq: c.seq,
        command,
    })
}

fn disposition_from_pb(d: PbDisposition) -> Result<Disposition, CodecError> {
    Ok(Disposition {
        ordinal: Ordinal(d.ordinal),
        source: source_from(d.session)?,
        seq: d.seq,
        tick_seen: Tick(d.tick_seen),
        reason: reason_from_pb(d.reason)?,
    })
}

fn kind_from_pb(v: i32) -> Result<EntityKind, CodecError> {
    match v {
        1 => Ok(EntityKind::Player),
        2 => Ok(EntityKind::Npc),
        other => Err(CodecError(format!("unknown entity kind {other}"))),
    }
}

fn output_from_pb(o: PbOutput) -> Result<ObserverOutput, CodecError> {
    Ok(match o.item {
        None => return Err(CodecError("output without an item".to_owned())),
        Some(PbOutputItem::Rejected(d)) => ObserverOutput::Rejected(disposition_from_pb(d)?),
        Some(PbOutputItem::Spawn(s)) => ObserverOutput::Event(ZoneEvent::EntitySpawn {
            tick: Tick(s.tick),
            entity: entity_from(&s.entity)?,
            kind: kind_from_pb(s.kind)?,
            name: s.name,
            pos: vec_from(s.pos)?,
            dest: s.dest.map(|d| vec_from(Some(d))).transpose()?,
            speed: Speed::from_milli_tiles_per_tick(s.speed),
            generation: SessionGeneration(s.generation),
        }),
        Some(PbOutputItem::Move(m)) => ObserverOutput::Event(ZoneEvent::EntityMove {
            tick: Tick(m.tick),
            entity: entity_from(&m.entity)?,
            pos: vec_from(m.pos)?,
            dest: m.dest.map(|d| vec_from(Some(d))).transpose()?,
            speed: Speed::from_milli_tiles_per_tick(m.speed),
        }),
        Some(PbOutputItem::Despawn(d)) => ObserverOutput::Event(ZoneEvent::EntityDespawn {
            tick: Tick(d.tick),
            entity: entity_from(&d.entity)?,
        }),
    })
}
