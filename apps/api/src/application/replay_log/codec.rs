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
//!   bytes events = 9;        // Outputs of every zone event (ENCODED) or its SHA-256 (SHA256)
//!   bytes state_digest = 10; // SHA-256 of the canonical end-of-tick state
//!   uint32 schema = 11;      // 3: JSON v1 digest; 4: binary v2 digest
//! }
//! enum OutputForm { ENCODED = 0; SHA256 = 1; }  // SHA256: outputs too large for one message
//! message Session { bytes entity = 1; uint64 generation = 2; }   // absent = System source
//! message Vec2 { sint32 x = 1; sint32 y = 2; }                     // milli-tiles
//! message Command {
//!   uint64 ordinal = 1; Session session = 2; optional uint32 seq = 3;
//!   oneof kind { SpawnPlayer spawn_player = 4; SpawnNpc spawn_npc = 5; EntityRef despawn = 6;
//!                Replace replace_session = 7; MoveTo move_to = 8; EntityRef stop_move = 9;
//!                SetTarget set_target = 10; EntityRef attack = 11;
//!                EntityRef stop_attack = 12; EntityRef respawn = 13; AddAggro add_aggro = 14; }
//! }
//! message SpawnPlayer { bytes entity = 1; string name = 2; Vec2 pos = 3; uint32 speed = 4;
//!                       uint64 generation = 5; PlayerLoad load = 6; }
//! message PlayerLoad { string class = 1; uint32 level = 2; uint64 xp = 3; optional uint32 hp = 4;
//!                      optional uint32 mp = 5; bool dead = 6; optional uint64 checkpoint_revision = 7; }
//! message SpawnNpc { string name = 1; Vec2 pos = 2; uint32 speed = 3; NpcCombat combat = 4; }
//! message NpcCombat { string template = 1; Stats stats = 2; sint32 attack_range = 3;
//!                     sint32 collision_radius = 4; uint64 xp_reward = 5; }
//! message Stats { uint32 level = 1; uint32 max_hp = 2; uint32 max_mp = 3; sint64 p_atk = 4;
//!                 sint64 p_def = 5; sint64 accuracy = 6; sint64 evasion = 7; uint32 crit = 8;
//!                 sint64 attack_speed = 9; uint32 random_damage = 10; }   // Q units
//! message AddAggro { bytes npc = 1; bytes target = 2; }
//! message SetTarget { bytes entity = 1; optional bytes target = 2;
//! }
//! message Disposition { uint64 ordinal = 1; Session session = 2; optional uint32 seq = 3;
//!                       uint64 tick_seen = 4; Reason reason = 5; }
//! message PlayerOutput { bytes entity = 1;
//!                        bytes encoded = 2;   // Outputs; set when output_form = ENCODED
//!                        bytes sha256 = 3; }  // SHA-256 of the Outputs bytes; when SHA256
//! message Outputs { repeated Output items = 1; }
//! message Output { oneof item { Spawn spawn = 1; Move move = 2; Despawn despawn = 3;
//!                               Disposition rejected = 4; Accepted accepted = 5;
//!                               AttackResult attack_result = 6; EntityDied entity_died = 7;
//!                               EntityRespawned entity_respawned = 8; StatsChanged stats_changed = 9;
//!                               XpGained xp_gained = 10; LevelUp level_up = 11;
//!                               TargetChanged target_changed = 12;
//!                               AttackStarted attack_started = 13;
//!                               AttackCancelled attack_cancelled = 14;
//!                               HateChanged hate_changed = 15;
//!                               NpcIntentionChanged npc_intention_changed = 16;
//!                               Progression progression = 17; } }
//! message NpcIntentionChanged { bytes entity = 1; uint64 tick = 2; Intention from = 3;
//!                               Intention to = 4; }
//! enum Intention { UNSPECIFIED = 0; IDLE = 1; ACTIVE = 2; ATTACK = 3; RETURN_HOME = 4; DEAD = 5; }
//! // Spawn gains `CombatView combat = 9`; AttackResult `target_incarnation = 7`;
//! // EntityDied `incarnation = 4`; EntityRespawned `incarnation = 5`; StatsChanged `xp = 8`.
//! message Progression { bytes entity = 1; uint64 tick = 2; uint32 level_before = 3;
//!   uint64 xp_before = 4; uint32 level = 5; uint64 xp = 6; uint32 hp = 7; uint32 mp = 8;
//!   bool alive = 9; Vec2 pos = 10; uint64 xp_gained = 11; repeated uint32 levels_gained = 12;
//!   DeathFact died = 13; bool respawned = 14; }
//! message DeathFact { optional bytes killer = 1; optional string killer_template = 2;
//!                     uint64 xp_lost = 3; }
//! message Accepted { uint32 seq = 1; uint64 tick = 2; uint64 ordinal = 3; }
//! message SessionIn  { bytes session = 1; uint64 seq = 2; uint32 zone = 3; uint64 epoch = 4;
//!                      uint64 tick_seen = 5; int64 recv_unix_ms = 6; bytes frame = 7; }
//! message SessionOut { bytes session = 1; uint32 zone = 2; uint64 epoch = 3; uint64 tick = 4;
//!                      bytes frame = 5; }
//! ```
//!
//! Record schema 5 selects `BinaryV3`. It adds command `bytes change_class = 15`,
//! `PlayerLoad.bytes progression = 8`, and output `bytes phase2 = 18`. These bytes are
//! canonical integer-only JSON of the new domain structs; `phase2` carries `ClassChanged`,
//! private `ClassTransfer`, and `Spawn`/`StatsChanged` when they contain Phase 2 projections.
//! Legacy variants continue through their original protobuf fields, byte for byte.
//!
//! Tag numbers and enum values are part of the log format: never renumber, only add.

#[path = "codec_sizing.rs"]
mod sizing;
pub use sizing::{
    command_admission_size, digest_record_upper_bound, validate_record_budget,
    AdmissionSizingError, CommandAdmissionSize,
};

#[path = "codec_item_views.rs"]
mod item_views;
#[path = "codec_items.rs"]
mod items;
pub(super) use item_views::from_json as item_view_from_json;
pub(super) use items::{grant_from_json, result_from_json};
#[path = "codec_phase3.rs"]
mod phase3;
#[path = "codec_phase5.rs"]
mod phase5;

use phase3::{PbAcceptResurrect, PbCancelCast, PbCastSkill, PbLearnSkill};

use bytes::Bytes;
use prost::Message;
use uuid::Uuid;

use super::record::{
    AppliedTickRecord, CodecError, OutputForm, PlayerOutput, SessionInRecord, SessionOutRecord,
};
use crate::domain::zone::{
    AppliedCommand, CombatView, CommandSource, DeathFact, Disposition, EntityId, EntityKind,
    FinalStats, Fixed, Intention, NpcCombat, ObserverOutput, Ordinal, Phase5CommandKind,
    Phase5OutputKind, PlayerLoad, ProgressionDelta, RejectReason, Scaled, SessionGeneration, Speed,
    StateDigestVersion, Swing, SwingCancel, Tick, Vec2Fixed, ZoneCommand, ZoneEvent, ZoneId,
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
    #[prost(bytes = "bytes", tag = "9")]
    events: Bytes,
    #[prost(bytes = "bytes", tag = "10")]
    state_digest: Bytes,
    #[prost(uint32, tag = "11")]
    schema: u32,
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
    #[prost(
        oneof = "PbCommandKind",
        tags = "4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76"
    )]
    kind: Option<PbCommandKind>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum PbCommandKind {
    // V7 Phase 5 economy intents and system jobs, design §12.2 (canonical JSON bodies).
    #[prost(bytes, tag = "32")]
    PickUpItem(Vec<u8>),
    #[prost(bytes, tag = "33")]
    VendorBuy(Vec<u8>),
    #[prost(bytes, tag = "34")]
    VendorSell(Vec<u8>),
    #[prost(bytes, tag = "35")]
    VendorRefund(Vec<u8>),
    #[prost(bytes, tag = "36")]
    MultisellChoose(Vec<u8>),
    #[prost(bytes, tag = "37")]
    RegisterRecipe(Vec<u8>),
    #[prost(bytes, tag = "38")]
    DeleteRecipe(Vec<u8>),
    #[prost(bytes, tag = "39")]
    CraftItem(Vec<u8>),
    #[prost(bytes, tag = "40")]
    TradeRequest(Vec<u8>),
    #[prost(bytes, tag = "41")]
    TradeAnswer(Vec<u8>),
    #[prost(bytes, tag = "42")]
    TradeAddItem(Vec<u8>),
    #[prost(bytes, tag = "43")]
    TradeConfirm(Vec<u8>),
    #[prost(bytes, tag = "44")]
    TradeCancel(Vec<u8>),
    #[prost(bytes, tag = "45")]
    SetPrivateStoreSell(Vec<u8>),
    #[prost(bytes, tag = "46")]
    SetPrivateStoreBuy(Vec<u8>),
    #[prost(bytes, tag = "47")]
    SetPrivateManufacture(Vec<u8>),
    #[prost(bytes, tag = "48")]
    ClosePrivateStore(Vec<u8>),
    #[prost(bytes, tag = "49")]
    PrivateStoreBuy(Vec<u8>),
    #[prost(bytes, tag = "50")]
    PrivateStoreSell(Vec<u8>),
    #[prost(bytes, tag = "51")]
    PrivateManufactureOrder(Vec<u8>),
    #[prost(bytes, tag = "52")]
    PartyInvite(Vec<u8>),
    #[prost(bytes, tag = "53")]
    PartyAnswer(Vec<u8>),
    #[prost(bytes, tag = "54")]
    PartyLeave(Vec<u8>),
    #[prost(bytes, tag = "55")]
    PartyKick(Vec<u8>),
    #[prost(bytes, tag = "56")]
    PartyChangeLeader(Vec<u8>),
    #[prost(bytes, tag = "57")]
    PartyLootChangeRequest(Vec<u8>),
    #[prost(bytes, tag = "58")]
    PartyLootChangeAnswer(Vec<u8>),
    #[prost(bytes, tag = "59")]
    WarehouseDeposit(Vec<u8>),
    #[prost(bytes, tag = "60")]
    WarehouseWithdraw(Vec<u8>),
    #[prost(bytes, tag = "61")]
    FreightSend(Vec<u8>),
    #[prost(bytes, tag = "62")]
    FreightWithdraw(Vec<u8>),
    #[prost(bytes, tag = "63")]
    MailSend(Vec<u8>),
    #[prost(bytes, tag = "64")]
    MailTakeAttachments(Vec<u8>),
    #[prost(bytes, tag = "65")]
    MailReject(Vec<u8>),
    #[prost(bytes, tag = "66")]
    MailCancel(Vec<u8>),
    #[prost(bytes, tag = "67")]
    MailDelete(Vec<u8>),
    #[prost(bytes, tag = "68")]
    ManorBuySeed(Vec<u8>),
    #[prost(bytes, tag = "69")]
    ManorSellCrop(Vec<u8>),
    #[prost(bytes, tag = "70")]
    ItemAuctionBid(Vec<u8>),
    #[prost(bytes, tag = "71")]
    ItemAuctionCancelBid(Vec<u8>),
    #[prost(bytes, tag = "72")]
    MailMarkRead(Vec<u8>),
    #[prost(bytes, tag = "73")]
    DeliverPending(Vec<u8>),
    #[prost(bytes, tag = "74")]
    AuctionSchedule(Vec<u8>),
    #[prost(bytes, tag = "75")]
    EpochReconcile(Vec<u8>),
    #[prost(bytes, tag = "76")]
    MailExpire(Vec<u8>),
    #[prost(bytes, tag = "30")]
    MaterializePendingItems(Vec<u8>),
    #[prost(bytes, tag = "31")]
    ExtendItemRange(Vec<u8>),
    #[prost(bytes, tag = "20")]
    EquipItem(Vec<u8>),
    #[prost(bytes, tag = "21")]
    UnequipItem(Vec<u8>),
    #[prost(bytes, tag = "22")]
    UseItem(Vec<u8>),
    #[prost(bytes, tag = "23")]
    DestroyItem(Vec<u8>),
    #[prost(bytes, tag = "24")]
    EnchantItem(Vec<u8>),
    #[prost(bytes, tag = "25")]
    ToggleAutoShot(Vec<u8>),
    #[prost(bytes, tag = "26")]
    RefineItem(Vec<u8>),
    #[prost(bytes, tag = "27")]
    CancelRefine(Vec<u8>),
    #[prost(bytes, tag = "28")]
    ExchangeSpecialAbility(Vec<u8>),
    #[prost(bytes, tag = "29")]
    CrystallizeItem(Vec<u8>),

    #[prost(message, tag = "16")]
    CastSkill(PbCastSkill),
    #[prost(message, tag = "17")]
    CancelCast(PbCancelCast),
    #[prost(message, tag = "18")]
    AcceptResurrect(PbAcceptResurrect),
    #[prost(message, tag = "19")]
    LearnSkill(PbLearnSkill),
    #[prost(bytes, tag = "15")]
    ChangeClass(Vec<u8>),
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
    #[prost(message, tag = "10")]
    SetTarget(PbSetTarget),
    #[prost(message, tag = "11")]
    Attack(PbEntityRef),
    #[prost(message, tag = "12")]
    StopAttack(PbEntityRef),
    #[prost(message, tag = "13")]
    Respawn(PbEntityRef),
    #[prost(message, tag = "14")]
    AddAggro(PbAddAggro),
}

#[derive(Clone, PartialEq, Message)]
struct PbAddAggro {
    #[prost(bytes = "vec", tag = "1")]
    npc: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    target: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct PbPlayerLoad {
    #[prost(bytes = "vec", optional, tag = "11")]
    phase5: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "10")]
    phase4: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "9")]
    phase3: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "8")]
    progression: Option<Vec<u8>>,
    #[prost(uint64, optional, tag = "7")]
    checkpoint_revision: Option<u64>,
    #[prost(string, tag = "1")]
    class: String,
    #[prost(uint32, tag = "2")]
    level: u32,
    #[prost(uint64, tag = "3")]
    xp: u64,
    #[prost(uint32, optional, tag = "4")]
    hp: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    mp: Option<u32>,
    #[prost(bool, tag = "6")]
    dead: bool,
}

#[derive(Clone, Copy, PartialEq, Message)]
struct PbStats {
    #[prost(uint32, tag = "1")]
    level: u32,
    #[prost(uint32, tag = "2")]
    max_hp: u32,
    #[prost(uint32, tag = "3")]
    max_mp: u32,
    #[prost(sint64, tag = "4")]
    p_atk: i64,
    #[prost(sint64, tag = "5")]
    p_def: i64,
    #[prost(sint64, tag = "6")]
    accuracy: i64,
    #[prost(sint64, tag = "7")]
    evasion: i64,
    #[prost(uint32, tag = "8")]
    crit: u32,
    #[prost(sint64, tag = "9")]
    attack_speed: i64,
    #[prost(uint32, tag = "10")]
    random_damage: u32,
}

#[derive(Clone, PartialEq, Message)]
struct PbNpcCombat {
    #[prost(string, tag = "1")]
    template: String,
    #[prost(message, optional, tag = "2")]
    stats: Option<PbStats>,
    #[prost(sint32, tag = "3")]
    attack_range: i32,
    #[prost(sint32, tag = "4")]
    collision_radius: i32,
    #[prost(uint64, tag = "5")]
    xp_reward: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbSwing {
    #[prost(bytes = "vec", tag = "1")]
    target: Vec<u8>,
    #[prost(uint32, tag = "2")]
    target_incarnation: u32,
    #[prost(uint64, tag = "3")]
    start: u64,
    #[prost(uint64, tag = "4")]
    impact: u64,
    #[prost(uint64, tag = "5")]
    ready: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbCombatView {
    #[prost(string, optional, tag = "1")]
    template: Option<String>,
    #[prost(uint32, tag = "2")]
    incarnation: u32,
    #[prost(bool, tag = "3")]
    dead: bool,
    #[prost(bool, tag = "4")]
    attackable: bool,
    #[prost(uint32, tag = "5")]
    hp: u32,
    #[prost(uint32, tag = "6")]
    max_hp: u32,
    #[prost(uint32, tag = "7")]
    level: u32,
    #[prost(message, optional, tag = "8")]
    swing: Option<PbSwing>,
}

#[derive(Clone, PartialEq, Message)]
struct PbAttackStarted {
    #[prost(bytes = "vec", tag = "1")]
    attacker: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    target: Vec<u8>,
    #[prost(uint64, tag = "3")]
    tick: u64,
    #[prost(uint32, tag = "4")]
    target_incarnation: u32,
    #[prost(uint64, tag = "5")]
    impact: u64,
    #[prost(uint64, tag = "6")]
    ready: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbAttackCancelled {
    #[prost(bytes = "vec", tag = "1")]
    attacker: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    target: Vec<u8>,
    #[prost(uint64, tag = "3")]
    tick: u64,
    #[prost(int32, tag = "4")]
    reason: i32,
}

#[derive(Clone, PartialEq, Message)]
struct PbHateChanged {
    #[prost(bytes = "vec", tag = "1")]
    npc: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    target: Vec<u8>,
    #[prost(uint64, tag = "3")]
    tick: u64,
    #[prost(uint64, tag = "4")]
    hate: u64,
    #[prost(uint64, tag = "5")]
    damage: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbNpcIntentionChanged {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    tick: u64,
    #[prost(int32, tag = "3")]
    from: i32,
    #[prost(int32, tag = "4")]
    to: i32,
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
    #[prost(message, optional, tag = "6")]
    load: Option<PbPlayerLoad>,
}

#[derive(Clone, PartialEq, Message)]
struct PbSpawnNpc {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, optional, tag = "2")]
    pos: Option<PbVec>,
    #[prost(uint32, tag = "3")]
    speed: u32,
    #[prost(message, optional, tag = "4")]
    combat: Option<PbNpcCombat>,
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
    #[prost(
        oneof = "PbOutputItem",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 52, 53, 54, 55"
    )]
    item: Option<PbOutputItem>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum PbOutputItem {
    // V7 Phase 5 carriers, design §12.3 (51 reserved: MailNotification is API-only).
    #[prost(bytes, tag = "42")]
    Phase5Enriched(Vec<u8>),
    #[prost(bytes, tag = "43")]
    EconomyResult(Vec<u8>),
    #[prost(bytes, tag = "44")]
    CorpseStateChanged(Vec<u8>),
    #[prost(bytes, tag = "45")]
    PartyStateChanged(Vec<u8>),
    #[prost(bytes, tag = "46")]
    PartyLootVote(Vec<u8>),
    #[prost(bytes, tag = "47")]
    PartyInviteReceived(Vec<u8>),
    #[prost(bytes, tag = "48")]
    TradeRequestReceived(Vec<u8>),
    #[prost(bytes, tag = "49")]
    TradeSessionChanged(Vec<u8>),
    #[prost(bytes, tag = "50")]
    PrivateStoreChanged(Vec<u8>),
    #[prost(bytes, tag = "52")]
    RecipeBookSync(Vec<u8>),
    #[prost(bytes, tag = "53")]
    ItemAuctionStatusChanged(Vec<u8>),
    #[prost(bytes, tag = "54")]
    LootDistributed(Vec<u8>),
    #[prost(bytes, tag = "55")]
    EconomyCheckpoint(Vec<u8>),
    #[prost(bytes, tag = "32")]
    InventorySync(Vec<u8>),
    #[prost(bytes, tag = "33")]
    InventoryChanged(Vec<u8>),
    #[prost(bytes, tag = "34")]
    EquipmentChanged(Vec<u8>),
    #[prost(bytes, tag = "35")]
    WeightChanged(Vec<u8>),
    #[prost(bytes, tag = "36")]
    PenaltyChanged(Vec<u8>),
    #[prost(bytes, tag = "37")]
    EnchantResult(Vec<u8>),
    #[prost(bytes, tag = "38")]
    RefineResult(Vec<u8>),
    #[prost(bytes, tag = "39")]
    ItemOperationResult(Vec<u8>),
    #[prost(bytes, tag = "40")]
    ItemCheckpoint(Vec<u8>),
    /// D26 V6-only enriched EntitySpawn/StatsChanged (canonical integer-only JSON).
    #[prost(bytes, tag = "41")]
    Phase4Enriched(Vec<u8>),
    #[prost(bytes, tag = "26")]
    PvpStateChanged(Vec<u8>),
    #[prost(bytes, tag = "27")]
    ResurrectOffer(Vec<u8>),
    #[prost(bytes, tag = "30")]
    SkillLearned(Vec<u8>),
    #[prost(bytes, tag = "23")]
    EffectApplied(Vec<u8>),
    #[prost(bytes, tag = "24")]
    EffectRemoved(Vec<u8>),
    #[prost(bytes, tag = "29")]
    SpGained(Vec<u8>),
    #[prost(bytes, tag = "20")]
    SkillCastStarted(Vec<u8>),
    #[prost(bytes, tag = "21")]
    SkillCastResult(Vec<u8>),
    #[prost(bytes, tag = "22")]
    SkillCastCancelled(Vec<u8>),
    #[prost(bytes, tag = "25")]
    SkillReuse(Vec<u8>),
    #[prost(bytes, tag = "31")]
    Phase3Projection(Vec<u8>),
    // Canonical integer-only Phase2 event JSON; legacy event tags remain byte-identical.
    #[prost(bytes, tag = "18")]
    Phase2(Vec<u8>),
    #[prost(bytes, tag = "19")]
    TokensReconciled(Vec<u8>),
    #[prost(message, tag = "1")]
    Spawn(PbSpawnEvent),
    #[prost(message, tag = "2")]
    Move(PbMoveEvent),
    #[prost(message, tag = "3")]
    Despawn(PbDespawnEvent),
    #[prost(message, tag = "4")]
    Rejected(PbDisposition),
    #[prost(message, tag = "5")]
    Accepted(PbAccepted),
    #[prost(message, tag = "6")]
    AttackResult(PbAttackResult),
    #[prost(message, tag = "7")]
    EntityDied(PbEntityDied),
    #[prost(message, tag = "8")]
    EntityRespawned(PbEntityRespawned),
    #[prost(message, tag = "9")]
    StatsChanged(PbStatsChanged),
    #[prost(message, tag = "10")]
    XpGained(PbXpGained),
    #[prost(message, tag = "11")]
    LevelUp(PbLevelUp),
    #[prost(message, tag = "12")]
    TargetChanged(PbTargetChanged),
    #[prost(message, tag = "13")]
    AttackStarted(PbAttackStarted),
    #[prost(message, tag = "14")]
    AttackCancelled(PbAttackCancelled),
    #[prost(message, tag = "15")]
    HateChanged(PbHateChanged),
    #[prost(message, tag = "16")]
    NpcIntentionChanged(PbNpcIntentionChanged),
    #[prost(message, tag = "17")]
    Progression(PbProgression),
}

#[derive(Clone, PartialEq, Message)]
struct PbAccepted {
    #[prost(uint64, tag = "3")]
    ordinal: u64,
    #[prost(uint32, tag = "1")]
    seq: u32,
    #[prost(uint64, tag = "2")]
    tick: u64,
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
    #[prost(message, optional, tag = "9")]
    combat: Option<PbCombatView>,
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
        events: r.events.clone(),
        state_digest: r.state_digest.clone(),
        schema: match r.digest_version {
            StateDigestVersion::JsonV1 => 3,
            StateDigestVersion::BinaryV2 => 4,
            StateDigestVersion::BinaryV3 => 5,
            StateDigestVersion::BinaryV4 => 6,
            StateDigestVersion::BinaryV5 => 7,
            StateDigestVersion::BinaryV6 => 8,
            StateDigestVersion::BinaryV7 => 9,
        },
    }
}

/// Version of the applied-tick record layout. 2 (Phase 1 E2.2): zone events, state digest,
/// combat commands and facts. 3 (E2.4/E2.6): progression facts, the
/// `alive` load flag, XP on `StatsChanged`. 4: binary v2 state digest. Schema 3 remains
/// readable and writable for JSON v1 replay; earlier versions are refused (1 reads 0).
/// 7: Phase 3 (`BinaryV5`). 8: Phase 4 items (`BinaryV6`). 9: Phase 5 economy (`BinaryV7`,
/// design §12.1): command kinds 32..76, player load 11 and outputs 42..55.
pub const RECORD_SCHEMA_VERSION: u32 = 9;

pub(super) fn encode_events(events: &[ZoneEvent]) -> Vec<u8> {
    PbOutputs {
        items: events
            .iter()
            .map(|e| PbOutput {
                item: Some(event_to_pb(e)),
            })
            .collect(),
    }
    .encode_to_vec()
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

#[derive(Clone, PartialEq, Message)]
struct PbSetTarget {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(bytes = "vec", optional, tag = "2")]
    target: Option<Vec<u8>>,
}

#[allow(clippy::too_many_lines)] // Exhaustive mapping preserves every historical command tag.
fn command_to_pb(c: &AppliedCommand) -> PbCommand {
    let kind = match &c.command {
        ZoneCommand::CastSkill {
            entity,
            skill_id,
            target,
        } => PbCommandKind::CastSkill(PbCastSkill {
            entity: entity_bytes(*entity),
            skill_id: skill_id.0,
            target: target.map(entity_bytes),
        }),
        ZoneCommand::CancelCast { entity, cast_id } => PbCommandKind::CancelCast(PbCancelCast {
            entity: entity_bytes(*entity),
            cast_id: cast_id.map(|id| id.0),
        }),
        ZoneCommand::AcceptResurrect {
            entity,
            offer_id,
            accept,
        } => PbCommandKind::AcceptResurrect(PbAcceptResurrect {
            entity: entity_bytes(*entity),
            offer_id: offer_id.0,
            accept: *accept,
        }),
        ZoneCommand::LearnSkill {
            entity,
            account,
            request_key,
            skill_id,
            skill_level,
        } => PbCommandKind::LearnSkill(PbLearnSkill {
            entity: entity_bytes(*entity),
            account: account.as_uuid().as_bytes().to_vec(),
            request_key: request_key.as_bytes().to_vec(),
            skill_id: skill_id.0,
            skill_level: *skill_level,
        }),
        ZoneCommand::MaterializePendingItems(command) => {
            PbCommandKind::MaterializePendingItems(canonical_json(command))
        },
        ZoneCommand::ExtendItemRange { range } => {
            PbCommandKind::ExtendItemRange(canonical_json(range))
        },
        ZoneCommand::Item(command) => items::command_to_pb(command),
        ZoneCommand::Phase5(command) => phase5::command_to_pb(command),
        ZoneCommand::ChangeClass { .. } => PbCommandKind::ChangeClass(canonical_json(&c.command)),
        ZoneCommand::SpawnPlayer {
            entity,
            name,
            pos,
            speed,
            generation,
            load,
        } => PbCommandKind::SpawnPlayer(PbSpawnPlayer {
            entity: entity_bytes(*entity),
            name: name.clone(),
            pos: Some(vec_to_pb(*pos)),
            speed: speed.milli_tiles_per_tick(),
            generation: generation.0,
            load: load.as_ref().map(|l| PbPlayerLoad {
                phase5: l.phase5.as_ref().map(canonical_json),
                phase4: l.phase4.as_ref().map(canonical_json),
                phase3: l.phase3.as_ref().map(canonical_json),
                progression: l.progression.as_ref().map(canonical_json),
                checkpoint_revision: l.checkpoint_revision,
                class: l.class.clone(),
                level: l.level,
                xp: l.xp,
                hp: l.hp,
                mp: l.mp,
                dead: !l.alive,
            }),
        }),
        ZoneCommand::SpawnNpc {
            name,
            pos,
            speed,
            combat,
        } => PbCommandKind::SpawnNpc(PbSpawnNpc {
            name: name.clone(),
            pos: Some(vec_to_pb(*pos)),
            speed: speed.milli_tiles_per_tick(),
            combat: combat.as_ref().map(|c| PbNpcCombat {
                template: c.template.clone(),
                stats: Some(stats_to_pb(&c.stats)),
                attack_range: c.attack_range.raw(),
                collision_radius: c.collision_radius.raw(),
                xp_reward: c.xp_reward,
            }),
        }),
        ZoneCommand::AddAggro { npc, target } => PbCommandKind::AddAggro(PbAddAggro {
            npc: entity_bytes(*npc),
            target: entity_bytes(*target),
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
        ZoneCommand::SetTarget { entity, target } => PbCommandKind::SetTarget(PbSetTarget {
            entity: entity_bytes(*entity),
            target: target.map(entity_bytes),
        }),
        ZoneCommand::Attack { entity } => PbCommandKind::Attack(PbEntityRef {
            entity: entity_bytes(*entity),
        }),
        ZoneCommand::StopAttack { entity } => PbCommandKind::StopAttack(PbEntityRef {
            entity: entity_bytes(*entity),
        }),
        ZoneCommand::Respawn { entity } => PbCommandKind::Respawn(PbEntityRef {
            entity: entity_bytes(*entity),
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
        RejectReason::TransferConflict => 16,
        RejectReason::TransferIneligible => 17,
        RejectReason::TransferRequirement => 18,
        RejectReason::InCombat => 19,
        RejectReason::ClassMasterTooFar => 20,
        RejectReason::ItemNotOwned => 39,
        RejectReason::ItemNotUsable => 40,
        RejectReason::InvalidEquipSlot => 41,
        RejectReason::InsufficientItems => 42,
        RejectReason::InventoryFull => 43,
        RejectReason::ItemGradeMismatch => 44,
        RejectReason::ItemReuse => 45,
        RejectReason::InvalidEnchant => 46,
        RejectReason::InvalidRefinement => 47,
        RejectReason::InvalidRecipe => 48,
        RejectReason::ItemOperationConflict => 49,
        RejectReason::ItemCapabilityDeferred => 50,
        RejectReason::CrystallizeLevelTooLow => 51,
        RejectReason::SkillUnknown => 21,
        RejectReason::SkillNotLearned => 22,
        RejectReason::SkillCooldown => 23,
        RejectReason::InsufficientMp => 24,
        RejectReason::InsufficientHp => 25,
        RejectReason::AlreadyCasting => 26,
        RejectReason::Incapacitated => 27,
        RejectReason::Silenced => 28,
        RejectReason::InvalidTargetType => 29,
        RejectReason::PeaceZone => 30,
        RejectReason::WeaponCondition => 31,
        RejectReason::SkillNotLearnable => 33,
        RejectReason::SkillWrongRank => 34,
        RejectReason::SkillLevelRequired => 35,
        RejectReason::SkillItemRequired => 36,
        RejectReason::SkillDeferred => 37,
        RejectReason::InsufficientSp => 38,
        RejectReason::NoResurrectOffer => 32,
        RejectReason::UnknownEntity => 1,
        RejectReason::OutOfBounds => 2,
        RejectReason::TooFar => 3,
        RejectReason::AlreadyExists => 4,
        RejectReason::NotPermitted => 5,
        RejectReason::StaleSession => 6,
        RejectReason::NotAPlayer => 7,
        RejectReason::DeadActor => 8,
        RejectReason::NonAttackableTarget => 9,
        RejectReason::TargetNotInAoi => 10,
        RejectReason::OutOfRange => 11,
        RejectReason::Protected => 12,
        RejectReason::NotYetImplemented => 13,
        RejectReason::InvalidLoad => 14,
        RejectReason::NotDead => 15,
    }
}

fn reason_from_pb(v: i32) -> Result<RejectReason, CodecError> {
    Ok(match v {
        16 => RejectReason::TransferConflict,
        17 => RejectReason::TransferIneligible,
        18 => RejectReason::TransferRequirement,
        19 => RejectReason::InCombat,
        20 => RejectReason::ClassMasterTooFar,
        39 => RejectReason::ItemNotOwned,
        40 => RejectReason::ItemNotUsable,
        41 => RejectReason::InvalidEquipSlot,
        42 => RejectReason::InsufficientItems,
        43 => RejectReason::InventoryFull,
        44 => RejectReason::ItemGradeMismatch,
        45 => RejectReason::ItemReuse,
        46 => RejectReason::InvalidEnchant,
        47 => RejectReason::InvalidRefinement,
        48 => RejectReason::InvalidRecipe,
        49 => RejectReason::ItemOperationConflict,
        50 => RejectReason::ItemCapabilityDeferred,
        51 => RejectReason::CrystallizeLevelTooLow,
        21 => RejectReason::SkillUnknown,
        22 => RejectReason::SkillNotLearned,
        23 => RejectReason::SkillCooldown,
        24 => RejectReason::InsufficientMp,
        25 => RejectReason::InsufficientHp,
        26 => RejectReason::AlreadyCasting,
        27 => RejectReason::Incapacitated,
        28 => RejectReason::Silenced,
        29 => RejectReason::InvalidTargetType,
        30 => RejectReason::PeaceZone,
        31 => RejectReason::WeaponCondition,
        33 => RejectReason::SkillNotLearnable,
        34 => RejectReason::SkillWrongRank,
        35 => RejectReason::SkillLevelRequired,
        36 => RejectReason::SkillItemRequired,
        37 => RejectReason::SkillDeferred,
        38 => RejectReason::InsufficientSp,
        32 => RejectReason::NoResurrectOffer,
        1 => RejectReason::UnknownEntity,
        2 => RejectReason::OutOfBounds,
        3 => RejectReason::TooFar,
        4 => RejectReason::AlreadyExists,
        5 => RejectReason::NotPermitted,
        6 => RejectReason::StaleSession,
        7 => RejectReason::NotAPlayer,
        8 => RejectReason::DeadActor,
        9 => RejectReason::NonAttackableTarget,
        10 => RejectReason::TargetNotInAoi,
        11 => RejectReason::OutOfRange,
        12 => RejectReason::Protected,
        13 => RejectReason::NotYetImplemented,
        14 => RejectReason::InvalidLoad,
        15 => RejectReason::NotDead,

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

#[derive(Clone, PartialEq, Message)]
struct PbAttackResult {
    #[prost(bytes = "vec", tag = "1")]
    attacker: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    target: Vec<u8>,
    #[prost(uint64, tag = "3")]
    tick: u64,
    #[prost(int32, tag = "4")]
    outcome: i32,
    #[prost(uint32, tag = "5")]
    damage: u32,
    #[prost(uint32, tag = "6")]
    target_hp_after: u32,
    #[prost(uint32, tag = "7")]
    target_incarnation: u32,
}

#[derive(Clone, PartialEq, Message)]
struct PbEntityDied {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    tick: u64,
    #[prost(bytes = "vec", optional, tag = "3")]
    killer: Option<Vec<u8>>,
    #[prost(uint32, tag = "4")]
    incarnation: u32,
}

#[derive(Clone, PartialEq, Message)]
struct PbEntityRespawned {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    tick: u64,
    #[prost(message, optional, tag = "3")]
    position: Option<PbVec>,
    #[prost(uint32, tag = "4")]
    hp: u32,
    #[prost(uint32, tag = "5")]
    incarnation: u32,
}

#[derive(Clone, PartialEq, Message)]
struct PbDeathFact {
    #[prost(bytes = "vec", optional, tag = "1")]
    killer: Option<Vec<u8>>,
    #[prost(string, optional, tag = "2")]
    killer_template: Option<String>,
    #[prost(uint64, tag = "3")]
    xp_lost: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbProgression {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    tick: u64,
    #[prost(uint32, tag = "3")]
    level_before: u32,
    #[prost(uint64, tag = "4")]
    xp_before: u64,
    #[prost(uint32, tag = "5")]
    level: u32,
    #[prost(uint64, tag = "6")]
    xp: u64,
    #[prost(uint32, tag = "7")]
    hp: u32,
    #[prost(uint32, tag = "8")]
    mp: u32,
    #[prost(bool, tag = "9")]
    alive: bool,
    #[prost(message, optional, tag = "10")]
    pos: Option<PbVec>,
    #[prost(uint64, tag = "11")]
    xp_gained: u64,
    #[prost(uint32, repeated, tag = "12")]
    levels_gained: Vec<u32>,
    #[prost(message, optional, tag = "13")]
    died: Option<PbDeathFact>,
    #[prost(bool, tag = "14")]
    respawned: bool,
}

#[derive(Clone, PartialEq, Message)]
struct PbStatsChanged {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint32, tag = "2")]
    hp: u32,
    #[prost(uint32, tag = "3")]
    max_hp: u32,
    #[prost(uint32, tag = "4")]
    mp: u32,
    #[prost(uint32, tag = "5")]
    max_mp: u32,
    #[prost(uint32, tag = "6")]
    level: u32,
    #[prost(uint64, tag = "7")]
    tick: u64,
    #[prost(uint64, tag = "8")]
    xp: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbXpGained {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    amount: u64,
    #[prost(uint64, tag = "3")]
    total: u64,
    #[prost(uint64, tag = "4")]
    tick: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbLevelUp {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(uint32, tag = "2")]
    level: u32,
    #[prost(uint64, tag = "3")]
    tick: u64,
}

#[derive(Clone, PartialEq, Message)]
struct PbTargetChanged {
    #[prost(bytes = "vec", tag = "1")]
    entity: Vec<u8>,
    #[prost(bytes = "vec", optional, tag = "2")]
    target: Option<Vec<u8>>,
    #[prost(uint64, tag = "3")]
    tick: u64,
}

fn output_to_pb(o: &ObserverOutput) -> PbOutput {
    let item = match o {
        ObserverOutput::Event(e) => event_to_pb(e),
        ObserverOutput::Accepted { ordinal, seq, tick } => PbOutputItem::Accepted(PbAccepted {
            ordinal: ordinal.0,
            seq: *seq,
            tick: tick.0,
        }),
        ObserverOutput::Rejected(d) => PbOutputItem::Rejected(disposition_to_pb(d)),
    };
    PbOutput { item: Some(item) }
}

fn stats_to_pb(s: &FinalStats) -> PbStats {
    PbStats {
        level: s.level,
        max_hp: s.max_hp,
        max_mp: s.max_mp,
        p_atk: s.p_atk.raw(),
        p_def: s.p_def.raw(),
        accuracy: s.accuracy.raw(),
        evasion: s.evasion.raw(),
        crit: s.crit_permille,
        attack_speed: s.attack_speed.raw(),
        random_damage: s.random_damage,
    }
}

fn swing_to_pb(s: &Swing) -> PbSwing {
    PbSwing {
        target: entity_bytes(s.target),
        target_incarnation: s.target_incarnation,
        start: s.start.0,
        impact: s.impact.0,
        ready: s.ready.0,
    }
}

const fn cancel_to_pb(r: SwingCancel) -> i32 {
    match r {
        SwingCancel::Stopped => 1,
        SwingCancel::TargetChanged => 2,
        SwingCancel::Moved => 3,
        SwingCancel::OutOfRange => 4,
        SwingCancel::TargetLost => 5,
        SwingCancel::AttackerDied => 6,
    }
}

const fn intention_to_pb(i: Intention) -> i32 {
    match i {
        Intention::Idle => 1,
        Intention::Active => 2,
        Intention::Attack => 3,
        Intention::ReturnHome => 4,
        Intention::Dead => 5,
    }
}

fn intention_from_pb(v: i32) -> Result<Intention, CodecError> {
    Ok(match v {
        1 => Intention::Idle,
        2 => Intention::Active,
        3 => Intention::Attack,
        4 => Intention::ReturnHome,
        5 => Intention::Dead,
        other => return Err(CodecError(format!("unknown intention {other}"))),
    })
}

fn cancel_from_pb(v: i32) -> Result<SwingCancel, CodecError> {
    Ok(match v {
        1 => SwingCancel::Stopped,
        2 => SwingCancel::TargetChanged,
        3 => SwingCancel::Moved,
        4 => SwingCancel::OutOfRange,
        5 => SwingCancel::TargetLost,
        6 => SwingCancel::AttackerDied,
        other => return Err(CodecError(format!("unknown swing cancel reason {other}"))),
    })
}

#[allow(clippy::too_many_lines)] // exhaustive one-to-one durable event mapping
fn event_to_pb(e: &ZoneEvent) -> PbOutputItem {
    // Design §12.3 tag 42, then D26: enriched carriers before every older dispatch.
    if phase5::enriched(e) {
        return PbOutputItem::Phase5Enriched(canonical_json(e));
    }
    if phase4_enriched(e) {
        return PbOutputItem::Phase4Enriched(canonical_json(e));
    }
    if phase3_carrier(e) {
        return PbOutputItem::Phase3Projection(canonical_json(e));
    }
    if matches!(
        e,
        ZoneEvent::EntitySpawn {
            identity: Some(_),
            ..
        } | ZoneEvent::StatsChanged { class: Some(_), .. }
    ) {
        return PbOutputItem::Phase2(canonical_json(e));
    }
    match e {
        ZoneEvent::Economy(event) => phase5::event_to_pb(event),
        ZoneEvent::EconomyCheckpoint(checkpoint) => {
            PbOutputItem::EconomyCheckpoint(canonical_json(checkpoint))
        },
        ZoneEvent::ItemView(view) => item_views::to_pb(view),
        ZoneEvent::ItemResult(result) => items::result_to_pb(result),
        ZoneEvent::ItemCheckpoint(projection) => {
            PbOutputItem::ItemCheckpoint(canonical_json(projection))
        },
        ZoneEvent::PvpStateChanged { .. } => PbOutputItem::PvpStateChanged(canonical_json(e)),
        ZoneEvent::ResurrectOffer { .. } => PbOutputItem::ResurrectOffer(canonical_json(e)),
        ZoneEvent::SkillLearned { .. } => PbOutputItem::SkillLearned(canonical_json(e)),
        ZoneEvent::EffectApplied { .. } => PbOutputItem::EffectApplied(canonical_json(e)),
        ZoneEvent::EffectRemoved { .. } => PbOutputItem::EffectRemoved(canonical_json(e)),
        ZoneEvent::SkillCastStarted { .. } => PbOutputItem::SkillCastStarted(canonical_json(e)),
        ZoneEvent::SkillCastResult { .. } => PbOutputItem::SkillCastResult(canonical_json(e)),
        ZoneEvent::SkillCastCancelled { .. } => PbOutputItem::SkillCastCancelled(canonical_json(e)),
        ZoneEvent::SpGained { .. } => PbOutputItem::SpGained(canonical_json(e)),
        ZoneEvent::SkillReuse { .. } => PbOutputItem::SkillReuse(canonical_json(e)),
        ZoneEvent::SkillStateSync { .. } | ZoneEvent::VitalsChanged { .. } => {
            PbOutputItem::Phase3Projection(canonical_json(e))
        },
        ZoneEvent::TokensReconciled { .. } => PbOutputItem::TokensReconciled(canonical_json(e)),
        ZoneEvent::ClassChanged { .. } | ZoneEvent::ClassTransfer { .. } => {
            PbOutputItem::Phase2(canonical_json(e))
        },
        ZoneEvent::AttackResult {
            attacker,
            target,
            tick,
            outcome,
            damage,
            target_hp_after,
            target_incarnation,
            phase3: _,
        } => PbOutputItem::AttackResult(PbAttackResult {
            attacker: entity_bytes(*attacker),
            target: entity_bytes(*target),
            tick: tick.0,
            outcome: match outcome {
                crate::domain::zone::AttackOutcome::Miss => 1,
                crate::domain::zone::AttackOutcome::Hit => 2,
                crate::domain::zone::AttackOutcome::Crit => 3,
            },
            damage: *damage,
            target_hp_after: *target_hp_after,
            target_incarnation: *target_incarnation,
        }),
        ZoneEvent::EntityDied {
            entity,
            tick,
            killer,
            incarnation,
        } => PbOutputItem::EntityDied(PbEntityDied {
            entity: entity_bytes(*entity),
            tick: tick.0,
            killer: killer.map(entity_bytes),
            incarnation: *incarnation,
        }),
        ZoneEvent::AttackStarted {
            tick,
            attacker,
            target,
            target_incarnation,
            impact,
            ready,
            phase3: _,
        } => PbOutputItem::AttackStarted(PbAttackStarted {
            attacker: entity_bytes(*attacker),
            target: entity_bytes(*target),
            tick: tick.0,
            target_incarnation: *target_incarnation,
            impact: impact.0,
            ready: ready.0,
        }),
        ZoneEvent::AttackCancelled {
            tick,
            attacker,
            target,
            reason,
        } => PbOutputItem::AttackCancelled(PbAttackCancelled {
            attacker: entity_bytes(*attacker),
            target: entity_bytes(*target),
            tick: tick.0,
            reason: cancel_to_pb(*reason),
        }),
        ZoneEvent::HateChanged {
            tick,
            npc,
            target,
            hate,
            damage,
        } => PbOutputItem::HateChanged(PbHateChanged {
            npc: entity_bytes(*npc),
            target: entity_bytes(*target),
            tick: tick.0,
            hate: *hate,
            damage: *damage,
        }),
        ZoneEvent::NpcIntentionChanged {
            tick,
            entity,
            from,
            to,
        } => PbOutputItem::NpcIntentionChanged(PbNpcIntentionChanged {
            entity: entity_bytes(*entity),
            tick: tick.0,
            from: intention_to_pb(*from),
            to: intention_to_pb(*to),
        }),
        ZoneEvent::EntityRespawned {
            entity,
            tick,
            position,
            hp,
            incarnation,
        } => PbOutputItem::EntityRespawned(PbEntityRespawned {
            entity: entity_bytes(*entity),
            tick: tick.0,
            position: Some(vec_to_pb(*position)),
            hp: *hp,
            incarnation: *incarnation,
        }),
        ZoneEvent::Progression(d) => PbOutputItem::Progression(PbProgression {
            entity: entity_bytes(d.entity),
            tick: d.tick.0,
            level_before: d.level_before,
            xp_before: d.xp_before,
            level: d.level,
            xp: d.xp,
            hp: d.hp,
            mp: d.mp,
            alive: d.alive,
            pos: Some(vec_to_pb(d.pos)),
            xp_gained: d.xp_gained,
            levels_gained: d.levels_gained.clone(),
            died: d.died.as_ref().map(|f| PbDeathFact {
                killer: f.killer.map(entity_bytes),
                killer_template: f.killer_template.clone(),
                xp_lost: f.xp_lost,
            }),
            respawned: d.respawned,
        }),
        ZoneEvent::StatsChanged {
            class: _,
            phase4: _,
            entity,
            hp,
            max_hp,
            mp,
            max_mp,
            level,
            xp,
            tick,
        } => PbOutputItem::StatsChanged(PbStatsChanged {
            entity: entity_bytes(*entity),
            hp: *hp,
            max_hp: *max_hp,
            mp: *mp,
            max_mp: *max_mp,
            level: *level,
            tick: tick.0,
            xp: *xp,
        }),
        ZoneEvent::XpGained {
            entity,
            amount,
            total,
            tick,
        } => PbOutputItem::XpGained(PbXpGained {
            entity: entity_bytes(*entity),
            amount: *amount,
            total: *total,
            tick: tick.0,
        }),
        ZoneEvent::LevelUp {
            entity,
            level,
            tick,
        } => PbOutputItem::LevelUp(PbLevelUp {
            entity: entity_bytes(*entity),
            level: *level,
            tick: tick.0,
        }),
        ZoneEvent::TargetChanged {
            entity,
            target,
            tick,
            phase3: _,
        } => PbOutputItem::TargetChanged(PbTargetChanged {
            entity: entity_bytes(*entity),
            target: target.map(entity_bytes),
            tick: tick.0,
        }),
        ZoneEvent::EntitySpawn {
            identity: _,
            phase4: _,
            phase5: _,
            tick,
            entity,
            kind,
            name,
            pos,
            dest,
            speed,
            generation,
            combat,
        } => PbOutputItem::Spawn(PbSpawnEvent {
            tick: tick.0,
            entity: entity_bytes(*entity),
            kind: kind_to_pb(*kind),
            name: name.clone(),
            pos: Some(vec_to_pb(*pos)),
            dest: dest.map(vec_to_pb),
            speed: speed.milli_tiles_per_tick(),
            generation: generation.0,
            combat: combat.as_ref().map(|c| PbCombatView {
                template: c.template.clone(),
                incarnation: c.incarnation,
                dead: c.dead,
                attackable: c.attackable,
                hp: c.hp,
                max_hp: c.max_hp,
                level: c.level,
                swing: c.swing.as_ref().map(swing_to_pb),
            }),
        }),
        ZoneEvent::EntityMove {
            tick,
            entity,
            pos,
            dest,
            speed,
        } => PbOutputItem::Move(PbMoveEvent {
            tick: tick.0,
            entity: entity_bytes(*entity),
            pos: Some(vec_to_pb(*pos)),
            dest: dest.map(vec_to_pb),
            speed: speed.milli_tiles_per_tick(),
        }),
        ZoneEvent::EntityDespawn { tick, entity } => PbOutputItem::Despawn(PbDespawnEvent {
            tick: tick.0,
            entity: entity_bytes(*entity),
        }),
    }
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

fn player_output_from_pb(
    o: PbPlayerOutput,
    form: OutputForm,
    phase4: bool,
) -> Result<PlayerOutput, CodecError> {
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
    let entity = entity_from(&o.entity)?;
    if phase4 && form == OutputForm::Encoded {
        for output in decode_outputs(&o.encoded)? {
            if let ObserverOutput::Event(ZoneEvent::ItemView(view)) = &output {
                if view.owner_only() && view.entity != entity {
                    return Err(CodecError("inventory in another owner's output".into()));
                }
            }
            if let ObserverOutput::Event(ZoneEvent::ItemResult(result)) = output {
                if result.entity != entity {
                    return Err(CodecError("item result in another owner's output".into()));
                }
            }
        }
    }
    let bytes = match form {
        OutputForm::Encoded => o.encoded,
        OutputForm::Sha256 => o.sha256,
    };
    Ok(PlayerOutput { entity, bytes })
}

pub(super) fn decode_record(bytes: &[u8]) -> Result<AppliedTickRecord, CodecError> {
    let pb = PbRecord::decode(bytes).map_err(err)?;
    if ![3, 4, 5, 6, 7, 8, RECORD_SCHEMA_VERSION].contains(&pb.schema) {
        return Err(CodecError(format!(
            "record schema {} is not supported (this build reads 3, 4, 5, 6, 7, 8 and {RECORD_SCHEMA_VERSION})",
            pb.schema
        )));
    }
    super::codec_validation::record(bytes, pb.schema, pb.output_form == 1)?;
    let output_form = output_form_from_pb(pb.output_form)?;
    let digest_len = match output_form {
        OutputForm::Encoded => None,
        OutputForm::Sha256 => Some(32),
    };
    if pb.state_digest.len() != 32 || digest_len.is_some_and(|n| pb.events.len() != n) {
        return Err(CodecError("record digests are malformed".to_owned()));
    }
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
            .map(|o| player_output_from_pb(o, output_form, pb.schema >= 8))
            .collect::<Result<_, _>>()?,
        output_form,
        events: pb.events,
        state_digest: pb.state_digest,
        digest_version: if pb.schema == 3 {
            StateDigestVersion::JsonV1
        } else if pb.schema == 4 {
            StateDigestVersion::BinaryV2
        } else if pb.schema == 5 {
            StateDigestVersion::BinaryV3
        } else if pb.schema == 6 {
            StateDigestVersion::BinaryV4
        } else if pb.schema == 7 {
            StateDigestVersion::BinaryV5
        } else if pb.schema == 8 {
            StateDigestVersion::BinaryV6
        } else {
            StateDigestVersion::BinaryV7
        },
    })
}

pub(super) fn decode_events(bytes: &[u8]) -> Result<Vec<ZoneEvent>, CodecError> {
    PbOutputs::decode(bytes)
        .map_err(err)?
        .items
        .into_iter()
        .map(|o| {
            event_from_pb(
                o.item
                    .ok_or_else(|| CodecError("event without an item".to_owned()))?,
            )
        })
        .collect()
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

#[allow(clippy::too_many_lines)] // One arm per recorded command kind keeps the dispatch exhaustive.
fn command_from_pb(c: PbCommand) -> Result<AppliedCommand, CodecError> {
    let command = match c.kind {
        Some(PbCommandKind::PickUpItem(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PickUpItem)?
        },
        Some(PbCommandKind::VendorBuy(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::VendorBuy)?
        },
        Some(PbCommandKind::VendorSell(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::VendorSell)?
        },
        Some(PbCommandKind::VendorRefund(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::VendorRefund)?
        },
        Some(PbCommandKind::MultisellChoose(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MultisellChoose)?
        },
        Some(PbCommandKind::RegisterRecipe(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::RegisterRecipe)?
        },
        Some(PbCommandKind::DeleteRecipe(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::DeleteRecipe)?
        },
        Some(PbCommandKind::CraftItem(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::CraftItem)?
        },
        Some(PbCommandKind::TradeRequest(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::TradeRequest)?
        },
        Some(PbCommandKind::TradeAnswer(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::TradeAnswer)?
        },
        Some(PbCommandKind::TradeAddItem(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::TradeAddItem)?
        },
        Some(PbCommandKind::TradeConfirm(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::TradeConfirm)?
        },
        Some(PbCommandKind::TradeCancel(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::TradeCancel)?
        },
        Some(PbCommandKind::SetPrivateStoreSell(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::SetPrivateStoreSell)?
        },
        Some(PbCommandKind::SetPrivateStoreBuy(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::SetPrivateStoreBuy)?
        },
        Some(PbCommandKind::SetPrivateManufacture(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::SetPrivateManufacture)?
        },
        Some(PbCommandKind::ClosePrivateStore(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::ClosePrivateStore)?
        },
        Some(PbCommandKind::PrivateStoreBuy(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PrivateStoreBuy)?
        },
        Some(PbCommandKind::PrivateStoreSell(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PrivateStoreSell)?
        },
        Some(PbCommandKind::PrivateManufactureOrder(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PrivateManufactureOrder)?
        },
        Some(PbCommandKind::PartyInvite(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyInvite)?
        },
        Some(PbCommandKind::PartyAnswer(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyAnswer)?
        },
        Some(PbCommandKind::PartyLeave(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyLeave)?
        },
        Some(PbCommandKind::PartyKick(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyKick)?
        },
        Some(PbCommandKind::PartyChangeLeader(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyChangeLeader)?
        },
        Some(PbCommandKind::PartyLootChangeRequest(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyLootChangeRequest)?
        },
        Some(PbCommandKind::PartyLootChangeAnswer(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::PartyLootChangeAnswer)?
        },
        Some(PbCommandKind::WarehouseDeposit(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::WarehouseDeposit)?
        },
        Some(PbCommandKind::WarehouseWithdraw(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::WarehouseWithdraw)?
        },
        Some(PbCommandKind::FreightSend(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::FreightSend)?
        },
        Some(PbCommandKind::FreightWithdraw(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::FreightWithdraw)?
        },
        Some(PbCommandKind::MailSend(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailSend)?
        },
        Some(PbCommandKind::MailTakeAttachments(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailTakeAttachments)?
        },
        Some(PbCommandKind::MailReject(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailReject)?
        },
        Some(PbCommandKind::MailCancel(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailCancel)?
        },
        Some(PbCommandKind::MailDelete(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailDelete)?
        },
        Some(PbCommandKind::ManorBuySeed(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::ManorBuySeed)?
        },
        Some(PbCommandKind::ManorSellCrop(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::ManorSellCrop)?
        },
        Some(PbCommandKind::ItemAuctionBid(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::ItemAuctionBid)?
        },
        Some(PbCommandKind::ItemAuctionCancelBid(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::ItemAuctionCancelBid)?
        },
        Some(PbCommandKind::MailMarkRead(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailMarkRead)?
        },
        Some(PbCommandKind::DeliverPending(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::DeliverPending)?
        },
        Some(PbCommandKind::AuctionSchedule(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::AuctionSchedule)?
        },
        Some(PbCommandKind::EpochReconcile(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::EpochReconcile)?
        },
        Some(PbCommandKind::MailExpire(bytes)) => {
            phase5::command_from_json(&bytes, Phase5CommandKind::MailExpire)?
        },
        Some(PbCommandKind::MaterializePendingItems(bytes)) => items::grant_from_json(&bytes)?,
        Some(PbCommandKind::ExtendItemRange(bytes)) => items::range_from_json(&bytes)?,
        Some(PbCommandKind::EquipItem(bytes)) => items::command_from_json(&bytes, 20)?,
        Some(PbCommandKind::UnequipItem(bytes)) => items::command_from_json(&bytes, 21)?,
        Some(PbCommandKind::UseItem(bytes)) => items::command_from_json(&bytes, 22)?,
        Some(PbCommandKind::DestroyItem(bytes)) => items::command_from_json(&bytes, 23)?,
        Some(PbCommandKind::EnchantItem(bytes)) => items::command_from_json(&bytes, 24)?,
        Some(PbCommandKind::ToggleAutoShot(bytes)) => items::command_from_json(&bytes, 25)?,
        Some(PbCommandKind::RefineItem(bytes)) => items::command_from_json(&bytes, 26)?,
        Some(PbCommandKind::CancelRefine(bytes)) => items::command_from_json(&bytes, 27)?,
        Some(PbCommandKind::ExchangeSpecialAbility(bytes)) => items::command_from_json(&bytes, 28)?,
        Some(PbCommandKind::CrystallizeItem(bytes)) => items::command_from_json(&bytes, 29)?,

        None => return Err(CodecError("command without a kind".to_owned())),
        Some(PbCommandKind::CastSkill(value)) => value.into_command()?,
        Some(PbCommandKind::CancelCast(value)) => value.into_command()?,
        Some(PbCommandKind::AcceptResurrect(value)) => value.into_command()?,
        Some(PbCommandKind::LearnSkill(value)) => value.into_command()?,
        Some(PbCommandKind::ChangeClass(bytes)) => {
            let command: ZoneCommand = serde_json::from_slice(&bytes).map_err(err)?;
            if !matches!(command, ZoneCommand::ChangeClass { .. }) {
                return Err(err("invalid class command tag"));
            }
            command
        },
        Some(PbCommandKind::SpawnPlayer(s)) => ZoneCommand::SpawnPlayer {
            entity: entity_from(&s.entity)?,
            name: s.name,
            pos: vec_from(s.pos)?,
            speed: Speed::from_milli_tiles_per_tick(s.speed),
            generation: SessionGeneration(s.generation),
            load: s
                .load
                .map(|l| {
                    let load = PlayerLoad {
                        phase5: l.phase5.map(|b| phase5::load_from_json(&b)).transpose()?,
                        phase4: l.phase4.map(|b| phase4_load_from_json(&b)).transpose()?,
                        phase3: l
                            .phase3
                            .map(|b| serde_json::from_slice(&b).map_err(err))
                            .transpose()?,
                        progression: l
                            .progression
                            .map(|b| serde_json::from_slice(&b).map_err(err))
                            .transpose()?,
                        checkpoint_revision: l.checkpoint_revision,
                        class: l.class,
                        level: l.level,
                        xp: l.xp,
                        hp: l.hp,
                        mp: l.mp,
                        alive: !l.dead,
                    };
                    load.validate_phase4().map_err(err)?;
                    Ok::<_, CodecError>(Box::new(load))
                })
                .transpose()?,
        },
        Some(PbCommandKind::SpawnNpc(s)) => ZoneCommand::SpawnNpc {
            name: s.name,
            pos: vec_from(s.pos)?,
            speed: Speed::from_milli_tiles_per_tick(s.speed),
            combat: s
                .combat
                .map(|c| {
                    Ok::<_, CodecError>(Box::new(NpcCombat {
                        template: c.template,
                        stats: stats_from_pb(
                            c.stats
                                .ok_or_else(|| CodecError("NPC combat without stats".to_owned()))?,
                        ),
                        attack_range: Fixed::from_raw(c.attack_range),
                        collision_radius: Fixed::from_raw(c.collision_radius),
                        xp_reward: c.xp_reward,
                    }))
                })
                .transpose()?,
        },
        Some(PbCommandKind::AddAggro(a)) => ZoneCommand::AddAggro {
            npc: entity_from(&a.npc)?,
            target: entity_from(&a.target)?,
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
        Some(PbCommandKind::SetTarget(r)) => ZoneCommand::SetTarget {
            entity: entity_from(&r.entity)?,
            target: r.target.map(|id| entity_from(&id)).transpose()?,
        },
        Some(PbCommandKind::Attack(r)) => ZoneCommand::Attack {
            entity: entity_from(&r.entity)?,
        },
        Some(PbCommandKind::StopAttack(r)) => ZoneCommand::StopAttack {
            entity: entity_from(&r.entity)?,
        },
        Some(PbCommandKind::Respawn(r)) => ZoneCommand::Respawn {
            entity: entity_from(&r.entity)?,
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
        Some(PbOutputItem::Accepted(a)) => ObserverOutput::Accepted {
            ordinal: Ordinal(a.ordinal),
            seq: a.seq,
            tick: Tick(a.tick),
        },
        Some(PbOutputItem::Rejected(d)) => ObserverOutput::Rejected(disposition_from_pb(d)?),
        Some(item) => ObserverOutput::Event(event_from_pb(item)?),
    })
}

fn swing_from_pb(s: &PbSwing) -> Result<Swing, CodecError> {
    Ok(Swing {
        target: entity_from(&s.target)?,
        target_incarnation: s.target_incarnation,
        start: Tick(s.start),
        impact: Tick(s.impact),
        ready: Tick(s.ready),
    })
}

fn stats_from_pb(s: PbStats) -> FinalStats {
    FinalStats {
        level: s.level,
        max_hp: s.max_hp,
        max_mp: s.max_mp,
        p_atk: Scaled::from_raw(s.p_atk),
        p_def: Scaled::from_raw(s.p_def),
        accuracy: Scaled::from_raw(s.accuracy),
        evasion: Scaled::from_raw(s.evasion),
        crit_permille: s.crit,
        attack_speed: Scaled::from_raw(s.attack_speed),
        random_damage: s.random_damage,
    }
}

#[allow(clippy::too_many_lines)] // exhaustive one-to-one durable event mapping
fn event_from_pb(item: PbOutputItem) -> Result<ZoneEvent, CodecError> {
    Ok(match item {
        PbOutputItem::Phase5Enriched(bytes) => phase5::enriched_from_json(&bytes)?,
        PbOutputItem::EconomyCheckpoint(bytes) => phase5::checkpoint_from_json(&bytes)?,
        PbOutputItem::EconomyResult(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::EconomyResult)?
        },
        PbOutputItem::CorpseStateChanged(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::CorpseStateChanged)?
        },
        PbOutputItem::PartyStateChanged(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::PartyStateChanged)?
        },
        PbOutputItem::PartyLootVote(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::PartyLootVote)?
        },
        PbOutputItem::PartyInviteReceived(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::PartyInviteReceived)?
        },
        PbOutputItem::TradeRequestReceived(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::TradeRequestReceived)?
        },
        PbOutputItem::TradeSessionChanged(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::TradeSessionChanged)?
        },
        PbOutputItem::PrivateStoreChanged(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::PrivateStoreChanged)?
        },
        PbOutputItem::RecipeBookSync(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::RecipeBookSync)?
        },
        PbOutputItem::ItemAuctionStatusChanged(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::ItemAuctionStatusChanged)?
        },
        PbOutputItem::LootDistributed(bytes) => {
            phase5::event_from_json(&bytes, Phase5OutputKind::LootDistributed)?
        },
        PbOutputItem::InventorySync(bytes) => item_views::from_json(&bytes, 32)?,
        PbOutputItem::InventoryChanged(bytes) => item_views::from_json(&bytes, 33)?,
        PbOutputItem::EquipmentChanged(bytes) => item_views::from_json(&bytes, 34)?,
        PbOutputItem::WeightChanged(bytes) => item_views::from_json(&bytes, 35)?,
        PbOutputItem::PenaltyChanged(bytes) => item_views::from_json(&bytes, 36)?,
        PbOutputItem::EnchantResult(bytes) => items::result_from_json(&bytes, 37)?,
        PbOutputItem::RefineResult(bytes) => items::result_from_json(&bytes, 38)?,
        PbOutputItem::ItemOperationResult(bytes) => items::result_from_json(&bytes, 39)?,
        PbOutputItem::ItemCheckpoint(bytes) => {
            ZoneEvent::ItemCheckpoint(Box::new(phase4_projection_from_json(&bytes)?))
        },
        PbOutputItem::PvpStateChanged(bytes) => {
            let event = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::PvpStateChanged { incarnation, .. } if incarnation != 0)
            {
                return Err(err("invalid Phase3 PvP fact"));
            }
            event
        },
        PbOutputItem::ResurrectOffer(bytes) => {
            let event = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::ResurrectOffer { target, offer_id, restore_percent_q, .. } if target.incarnation != 0 && offer_id.0 != 0 && (0..=100_000_000).contains(&restore_percent_q))
            {
                return Err(err("invalid Phase3 resurrection fact"));
            }
            event
        },
        PbOutputItem::SkillLearned(bytes) => {
            let event = phase3_event_from_json(&bytes)?;
            if !matches!(&event, ZoneEvent::SkillLearned { tick, entity, result, .. }
                if result.tick == *tick && result.character_id.as_uuid() == entity.as_uuid()
                    && result.skill.id.0 != 0 && result.skill.rank != 0 && result.incarnation != 0
                    && result.hp <= result.max_hp && result.mp <= result.max_mp && result.cp <= result.max_cp
                    && result.sp_remaining <= i64::MAX.unsigned_abs() && result.sp_spent <= i64::MAX.unsigned_abs())
            {
                return Err(err("invalid Phase3 learning fact"));
            }
            event
        },
        PbOutputItem::EffectApplied(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::EffectApplied { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::EffectRemoved(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::EffectRemoved { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::SkillCastStarted(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::SkillCastStarted { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::SkillCastResult(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::SkillCastResult { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::SkillCastCancelled(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::SkillCastCancelled { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::SpGained(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::SpGained { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::SkillReuse(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if !matches!(event, ZoneEvent::SkillReuse { .. }) {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::Phase4Enriched(bytes) => enriched_from_json(&bytes)?,
        PbOutputItem::Phase3Projection(bytes) => {
            let event: ZoneEvent = phase3_event_from_json(&bytes)?;
            if phase4_enriched(&event) || phase5::enriched(&event) {
                return Err(err("Phase4 enrichment in an older carrier"));
            }
            if !matches!(event, ZoneEvent::VitalsChanged { .. } | ZoneEvent::SkillStateSync { .. })
                && !phase3_carrier(&event)
            {
                return Err(err("invalid Phase3 event tag"));
            }
            event
        },
        PbOutputItem::TokensReconciled(bytes) => {
            let event: ZoneEvent = serde_json::from_slice(&bytes).map_err(err)?;
            if !matches!(event, ZoneEvent::TokensReconciled { adjustment, .. }
                if adjustment.claimed_mask != 0 && adjustment.claimed_mask & !3 == 0
                    && adjustment.granted_mask & !adjustment.claimed_mask == 0)
            {
                return Err(err("invalid token reconciliation tag"));
            }
            event
        },
        PbOutputItem::Phase2(bytes) => {
            let event: ZoneEvent = serde_json::from_slice(&bytes).map_err(err)?;
            if phase4_enriched(&event) || phase5::enriched(&event) {
                return Err(err("Phase4 enrichment in an older carrier"));
            }
            if !matches!(
                event,
                ZoneEvent::ClassChanged { .. }
                    | ZoneEvent::ClassTransfer { .. }
                    | ZoneEvent::EntitySpawn {
                        identity: Some(_),
                        ..
                    }
                    | ZoneEvent::StatsChanged { class: Some(_), .. }
            ) {
                return Err(err("invalid Phase2 event tag"));
            }
            event
        },
        PbOutputItem::Accepted(_) | PbOutputItem::Rejected(_) => {
            return Err(CodecError("response where an event was expected".to_owned()))
        },
        PbOutputItem::AttackStarted(e) => ZoneEvent::AttackStarted {
            phase3: None,
            tick: Tick(e.tick),
            attacker: entity_from(&e.attacker)?,
            target: entity_from(&e.target)?,
            target_incarnation: e.target_incarnation,
            impact: Tick(e.impact),
            ready: Tick(e.ready),
        },
        PbOutputItem::AttackCancelled(e) => ZoneEvent::AttackCancelled {
            tick: Tick(e.tick),
            attacker: entity_from(&e.attacker)?,
            target: entity_from(&e.target)?,
            reason: cancel_from_pb(e.reason)?,
        },
        PbOutputItem::HateChanged(e) => ZoneEvent::HateChanged {
            tick: Tick(e.tick),
            npc: entity_from(&e.npc)?,
            target: entity_from(&e.target)?,
            hate: e.hate,
            damage: e.damage,
        },
        PbOutputItem::NpcIntentionChanged(e) => ZoneEvent::NpcIntentionChanged {
            tick: Tick(e.tick),
            entity: entity_from(&e.entity)?,
            from: intention_from_pb(e.from)?,
            to: intention_from_pb(e.to)?,
        },
        PbOutputItem::AttackResult(e) => ZoneEvent::AttackResult {
            phase3: None,
            attacker: entity_from(&e.attacker)?,
            target: entity_from(&e.target)?,
            tick: Tick(e.tick),
            outcome: match e.outcome {
                1 => crate::domain::zone::AttackOutcome::Miss,
                2 => crate::domain::zone::AttackOutcome::Hit,
                3 => crate::domain::zone::AttackOutcome::Crit,
                other => return Err(CodecError(format!("unknown attack outcome {other}"))),
            },
            damage: e.damage,
            target_hp_after: e.target_hp_after,
            target_incarnation: e.target_incarnation,
        },
        PbOutputItem::EntityDied(e) => ZoneEvent::EntityDied {
            entity: entity_from(&e.entity)?,
            tick: Tick(e.tick),
            killer: e.killer.map(|id| entity_from(&id)).transpose()?,
            incarnation: e.incarnation,
        },
        PbOutputItem::EntityRespawned(e) => ZoneEvent::EntityRespawned {
            entity: entity_from(&e.entity)?,
            tick: Tick(e.tick),
            position: vec_from(e.position)?,
            hp: e.hp,
            incarnation: e.incarnation,
        },
        PbOutputItem::Progression(d) => ZoneEvent::Progression(ProgressionDelta {
            tick: Tick(d.tick),
            entity: entity_from(&d.entity)?,
            level_before: d.level_before,
            xp_before: d.xp_before,
            level: d.level,
            xp: d.xp,
            hp: d.hp,
            mp: d.mp,
            alive: d.alive,
            pos: vec_from(d.pos)?,
            xp_gained: d.xp_gained,
            levels_gained: d.levels_gained,
            died: d
                .died
                .map(|f| {
                    Ok::<_, CodecError>(DeathFact {
                        killer: f.killer.map(|id| entity_from(&id)).transpose()?,
                        killer_template: f.killer_template,
                        xp_lost: f.xp_lost,
                    })
                })
                .transpose()?,
            respawned: d.respawned,
        }),
        PbOutputItem::StatsChanged(e) => ZoneEvent::StatsChanged {
            phase4: None,
            class: None,
            entity: entity_from(&e.entity)?,
            hp: e.hp,
            max_hp: e.max_hp,
            mp: e.mp,
            max_mp: e.max_mp,
            level: e.level,
            xp: e.xp,
            tick: Tick(e.tick),
        },
        PbOutputItem::XpGained(e) => ZoneEvent::XpGained {
            entity: entity_from(&e.entity)?,
            amount: e.amount,
            total: e.total,
            tick: Tick(e.tick),
        },
        PbOutputItem::LevelUp(e) => ZoneEvent::LevelUp {
            entity: entity_from(&e.entity)?,
            level: e.level,
            tick: Tick(e.tick),
        },
        PbOutputItem::TargetChanged(e) => ZoneEvent::TargetChanged {
            phase3: None,
            entity: entity_from(&e.entity)?,
            target: e.target.map(|id| entity_from(&id)).transpose()?,
            tick: Tick(e.tick),
        },
        PbOutputItem::Spawn(s) => ZoneEvent::EntitySpawn {
            phase4: None,
            phase5: None,
            identity: None,
            tick: Tick(s.tick),
            entity: entity_from(&s.entity)?,
            kind: kind_from_pb(s.kind)?,
            name: s.name,
            pos: vec_from(s.pos)?,
            dest: s.dest.map(|d| vec_from(Some(d))).transpose()?,
            speed: Speed::from_milli_tiles_per_tick(s.speed),
            generation: SessionGeneration(s.generation),
            combat: s
                .combat
                .map(|c| {
                    Ok::<_, CodecError>(CombatView {
                        phase3: None,
                        template: c.template,
                        incarnation: c.incarnation,
                        dead: c.dead,
                        attackable: c.attackable,
                        hp: c.hp,
                        max_hp: c.max_hp,
                        level: c.level,
                        swing: c.swing.as_ref().map(swing_from_pb).transpose()?,
                    })
                })
                .transpose()?,
        },
        PbOutputItem::Move(m) => ZoneEvent::EntityMove {
            tick: Tick(m.tick),
            entity: entity_from(&m.entity)?,
            pos: vec_from(m.pos)?,
            dest: m.dest.map(|d| vec_from(Some(d))).transpose()?,
            speed: Speed::from_milli_tiles_per_tick(m.speed),
        },
        PbOutputItem::Despawn(d) => ZoneEvent::EntityDespawn {
            tick: Tick(d.tick),
            entity: entity_from(&d.entity)?,
        },
    })
}

// V5 event slots have one canonical representation. Check after decoding so serde cannot
// silently discard an unknown nested extension or normalize a duplicate map key. Legacy
// slots retain their established grammar and byte representation.
fn phase3_carrier(event: &ZoneEvent) -> bool {
    matches!(
        event,
        ZoneEvent::AttackResult {
            phase3: Some(_),
            ..
        } | ZoneEvent::AttackStarted {
            phase3: Some(_),
            ..
        } | ZoneEvent::TargetChanged {
            phase3: Some(_),
            ..
        }
    ) || matches!(event, ZoneEvent::EntitySpawn { combat: Some(c), .. } if c.phase3.is_some())
        || matches!(event, ZoneEvent::StatsChanged { class: Some(c), .. } if c.phase3.is_some())
}

/// D26: exactly EntitySpawn/StatsChanged carrying their Phase4 sidecar.
fn phase4_enriched(event: &ZoneEvent) -> bool {
    matches!(
        event,
        ZoneEvent::EntitySpawn {
            phase4: Some(_),
            ..
        } | ZoneEvent::StatsChanged {
            phase4: Some(_),
            ..
        }
    )
}

/// Strict tag41 grammar: canonical bytes, permitted variants, complete sidecars. A spawn
/// needs its public combat Phase3 policy; owner stats need the owner Phase3 sheet whose
/// nested effective projection the Phase4 extension completes.
pub(super) fn enriched_from_json(bytes: &[u8]) -> Result<ZoneEvent, CodecError> {
    let event = phase3_event_from_json(bytes)?;
    if phase5::enriched(&event) {
        return Err(err("Phase5 enrichment in an older carrier"));
    }
    let valid = if let ZoneEvent::EntitySpawn {
        phase4: Some(items),
        combat: Some(combat),
        identity: Some(_),
        ..
    } = &event
    {
        combat.phase3.is_some() && items.validate().is_ok()
    } else if let ZoneEvent::StatsChanged {
        phase4: Some(items),
        class: Some(class),
        ..
    } = &event
    {
        class.phase3.is_some() && items.validate().is_ok()
    } else {
        false
    };
    if !valid {
        return Err(err("invalid Phase4 enriched event"));
    }
    Ok(event)
}

fn phase3_event_from_json(bytes: &[u8]) -> Result<ZoneEvent, CodecError> {
    let event: ZoneEvent = serde_json::from_slice(bytes).map_err(err)?;
    if canonical_json(&event) != bytes {
        return Err(err("noncanonical Phase3 event payload"));
    }
    Ok(event)
}

// These plain structs contain only bounded integer/string data and ordered vectors. JSON
// serialization cannot fail; using one canonical encoding retains absent legacy fields.
/// Validate opaque private DTO bytes while traversing the enclosing record grammar.
pub(super) fn phase4_projection_from_json(
    bytes: &[u8],
) -> Result<crate::domain::zone::Phase4Projection, CodecError> {
    use crate::domain::zone::phase4_checkpoint::RetainedEquipmentSource;
    let projection: crate::domain::zone::Phase4Projection =
        serde_json::from_slice(bytes).map_err(err)?;
    let count = projection
        .checkpoint
        .equipment_install_order
        .iter()
        .filter(|source| matches!(source, RetainedEquipmentSource::Phase3(_)))
        .count();
    projection.validate_structure(count).map_err(err)?;
    if canonical_json(&projection) != bytes {
        return Err(err("noncanonical private item projection"));
    }
    Ok(projection)
}

pub(super) fn phase4_load_from_json(
    bytes: &[u8],
) -> Result<crate::domain::zone::Phase4PlayerLoad, CodecError> {
    let load = serde_json::from_slice(bytes).map_err(err)?;
    if canonical_json(&load) != bytes {
        return Err(err("noncanonical item admission load"));
    }
    Ok(load)
}

/// Record-grammar hook: the Phase 5 carriers' strict body checks (design §12.2, §12.3),
/// run while the raw record is traversed and again by the permissive decoder.
pub(super) fn phase5_payload(node: &str, tag: u32, payload: &[u8]) -> Result<(), CodecError> {
    match (node, tag) {
        ("PbCommand", tag) => {
            let kind = Phase5CommandKind::from_codec_tag(tag)
                .ok_or_else(|| err("unknown Phase 5 command tag"))?;
            phase5::command_from_json(payload, kind).map(|_| ())
        },
        ("PbPlayerLoad", 11) => phase5::load_from_json(payload).map(|_| ()),
        ("PbOutput", 42) => phase5::enriched_from_json(payload).map(|_| ()),
        ("PbOutput", 55) => phase5::checkpoint_from_json(payload).map(|_| ()),
        ("PbOutput", tag) => {
            let kind = Phase5OutputKind::from_codec_tag(tag)
                .ok_or_else(|| err("unknown Phase 5 output tag"))?;
            phase5::event_from_json(payload, kind).map(|_| ())
        },
        _ => Err(err("not a Phase 5 carrier")),
    }
}

fn canonical_json<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod combat_tests {
    #[test]
    fn unknown_record_schema_is_rejected_clearly() {
        let pb = super::PbRecord {
            schema: 999,
            ..Default::default()
        };
        let error = super::decode_record(&prost::Message::encode_to_vec(&pb)).unwrap_err();
        assert_eq!(error.to_string(), "cannot decode replay-log record: record schema 999 is not supported (this build reads 3, 4, 5, 6, 7, 8 and 9)");
    }

    use super::*;
    use crate::domain::zone::AttackOutcome;
    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn attack_result_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::AttackResult {
            phase3: None,
            attacker: id(1),
            target: id(2),
            tick: Tick(31),
            outcome: AttackOutcome::Crit,
            damage: 25,
            target_hp_after: 26,
            target_incarnation: 4,
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    #[test]
    fn entity_died_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::EntityDied {
            entity: id(1),
            tick: Tick(31),
            killer: Some(id(3)),
            incarnation: 2,
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    #[test]
    fn entity_respawned_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::EntityRespawned {
            entity: id(1),
            tick: Tick(31),
            position: Vec2Fixed::from_tiles(23, 24),
            hp: 24,
            incarnation: 3,
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    #[test]
    fn stats_changed_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::StatsChanged {
            phase4: None,
            class: None,
            tick: Tick(31),
            entity: id(1),
            hp: 22,
            max_hp: 23,
            mp: 24,
            max_mp: 25,
            level: 26,
            xp: u64::MAX,
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    fn enriched_events() -> (ZoneEvent, ZoneEvent, ZoneEvent) {
        use crate::domain::zone::item_views::{
            ItemPenaltyView, ItemWeightView, Phase4EffectiveView, Phase4SpawnView, Phase4StatsView,
            VisibleEquipment,
        };
        let combat = crate::domain::zone::CombatView {
            phase3: Some(crate::domain::zone::CombatViewPhase3 {
                status_flags: crate::domain::zone::effects::StatusFlags::default(),
                flag_until: None,
                karma: 0,
            }),
            template: None,
            incarnation: 2,
            dead: false,
            attackable: false,
            hp: 40,
            max_hp: 50,
            level: 3,
            swing: None,
        };
        let spawn = |visible: Vec<VisibleEquipment>| ZoneEvent::EntitySpawn {
            identity: Some(crate::domain::zone::PlayerIdentityView {
                race: crate::domain::Race::Human,
                class_id: crate::domain::class::ClassId(10),
                appearance: crate::domain::character_progression::CharacterAppearance::default(),
            }),
            phase4: Some(Box::new(Phase4SpawnView { visible })),
            phase5: None,
            tick: Tick(31),
            entity: id(1),
            kind: crate::domain::zone::EntityKind::Player,
            name: "Owner".into(),
            pos: Vec2Fixed::from_tiles(3, 4),
            dest: None,
            speed: crate::domain::zone::Speed::DEFAULT,
            generation: crate::domain::zone::SessionGeneration(1),
            combat: Some(combat.clone()),
        };
        let stats = ZoneEvent::StatsChanged {
            class: Some(crate::domain::zone::ClassStatsView {
                phase3: Some(Box::new(crate::domain::zone::OwnerPhase3View {
                    incarnation: 2,
                    baseline: Some(Tick(31)),
                    pvp: crate::domain::zone::PvpState {
                        flag_until: None,
                        karma: 0,
                        pk_count: 0,
                        pvp_count: 0,
                    },
                    death_penalty_level: 0,
                    effective: crate::domain::zone::LearnedEffectiveStats {
                        p_atk_q: 1,
                        p_def_q: 2,
                        m_atk_q: 3,
                        m_def_q: 4,
                        attack_speed: 300,
                        cast_speed: 333,
                        speed_q: 5,
                        accuracy_q: 6,
                        evasion_q: 7,
                        crit_permille: 8,
                    },
                })),
                class_id: crate::domain::class::ClassId(10),
                sp: 0,
                cp: 0,
                max_cp: 0,
                token_tier_1_count: 0,
                token_tier_2_count: 0,
            }),
            phase4: Some(Box::new(Phase4StatsView {
                weight: ItemWeightView::new(700, 1000, 1).unwrap(),
                penalties: ItemPenaltyView {
                    expertise: 0,
                    weapon_level: 1,
                    armor_level: 0,
                    overload_level: 2,
                    overloaded: true,
                },
                inventory_revision: 9,
                // Nondefault shield/reach/angle values and an explicit zero.
                effective: Phase4EffectiveView {
                    shield_def_q: 230_000_000,
                    shield_rate_q: 20_000_000,
                    shield_angle_degrees: 120,
                    physical_reach_millitiles: 2_500,
                    physical_angle_degrees: 0,
                },
            })),
            tick: Tick(31),
            entity: id(1),
            hp: 40,
            max_hp: 50,
            mp: 6,
            max_mp: 7,
            level: 3,
            xp: 99,
        };
        let visible = VisibleEquipment {
            slot: crate::domain::zone::EquipSlot::Rhand,
            template_id: 71,
            glow_tier: 1,
        };
        (spawn(vec![visible]), spawn(Vec::new()), stats)
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        clippy::indexing_slicing,
        clippy::wildcard_enum_match_arm,
        clippy::unreachable
    )]
    fn d26_enriched_events_use_tag41_round_trip_and_reject_partial_or_smuggled_sidecars() {
        use super::phase3_boundary_tests::{empty, nested};
        let (equipped, empty_gear, stats) = enriched_events();
        for event in [&equipped, &empty_gear, &stats] {
            let output = ObserverOutput::Event(event.clone());
            let pb = output_to_pb(&output);
            assert!(matches!(pb.item, Some(PbOutputItem::Phase4Enriched(_))), "{event:?}");
            let encoded = pb.encode_to_vec();
            let decoded = output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap();
            assert_eq!(decoded, output);
            assert_eq!(output_to_pb(&decoded).encode_to_vec(), encoded);
            // Record8 grammar accepts it; Record7 has no tag41.
            let outputs = encode_outputs(std::slice::from_ref(&output));
            let mut record = empty(8).encode_to_vec();
            record.extend(nested(7, &nested(2, &outputs)));
            assert!(super::super::codec_validation::record(&record, 8, false).is_ok());
            let mut legacy = empty(7).encode_to_vec();
            legacy.extend(nested(7, &nested(2, &outputs)));
            assert!(super::super::codec_validation::record(&legacy, 7, false).is_err());
        }
        // Phase4-absent events keep their exact older carriers.
        for event in [&equipped, &stats] {
            let mut legacy = event.clone();
            match &mut legacy {
                ZoneEvent::EntitySpawn { phase4, .. } => *phase4 = None,
                ZoneEvent::StatsChanged { phase4, .. } => *phase4 = None,
                _ => unreachable!(),
            }
            let item = output_to_pb(&ObserverOutput::Event(legacy)).item.unwrap();
            assert!(matches!(item, PbOutputItem::Phase3Projection(_)));
        }
        // Partial sidecars, wrong variants, noncanonical bytes and older carriers reject.
        let mut no_policy = equipped.clone();
        if let ZoneEvent::EntitySpawn {
            combat: Some(c), ..
        } = &mut no_policy
        {
            c.phase3 = None;
        }
        let mut no_sheet = stats.clone();
        if let ZoneEvent::StatsChanged { class: Some(c), .. } = &mut no_sheet {
            c.phase3 = None;
        }
        let mut tombstone = equipped.clone();
        if let ZoneEvent::EntitySpawn {
            phase4: Some(p), ..
        } = &mut tombstone
        {
            p.visible[0].template_id = 0;
        }
        let mut weight = stats.clone();
        if let ZoneEvent::StatsChanged {
            phase4: Some(p), ..
        } = &mut weight
        {
            p.weight.per_mille += 1;
        }
        let vitals = ZoneEvent::VitalsChanged {
            tick: Tick(31),
            entity: id(1),
            incarnation: 2,
            hp: 1,
            max_hp: 2,
            baseline: None,
        };
        for invalid in [&no_policy, &no_sheet, &tombstone, &weight, &vitals] {
            assert!(enriched_from_json(&canonical_json(invalid)).is_err(), "{invalid:?}");
        }
        let mut spaced = canonical_json(&equipped);
        spaced.insert(1, b' ');
        assert!(enriched_from_json(&spaced).is_err());
        let mut unknown = serde_json::to_value(&stats).unwrap();
        unknown["phase4"]["extra"] = serde_json::json!(1);
        assert!(enriched_from_json(&serde_json::to_vec(&unknown).unwrap()).is_err());
        for event in [&equipped, &stats] {
            assert!(event_from_pb(PbOutputItem::Phase2(canonical_json(event))).is_err());
            assert!(event_from_pb(PbOutputItem::Phase3Projection(canonical_json(event))).is_err());
        }
    }

    #[test]
    fn xp_gained_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::XpGained {
            tick: Tick(31),
            entity: id(1),
            amount: u64::MAX,
            total: u64::MAX,
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    #[test]
    fn level_up_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::LevelUp {
            tick: Tick(31),
            entity: id(1),
            level: 22,
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    #[test]
    fn target_changed_durable_round_trip() {
        let event = ObserverOutput::Event(ZoneEvent::TargetChanged {
            phase3: None,
            tick: Tick(31),
            entity: id(1),
            target: Some(id(2)),
        });
        let encoded = output_to_pb(&event).encode_to_vec();
        assert_eq!(output_from_pb(PbOutput::decode(encoded.as_slice()).unwrap()).unwrap(), event);
    }

    #[test]
    fn combat_commands_and_reasons_durable_round_trip() {
        for command in [
            ZoneCommand::SetTarget {
                entity: id(1),
                target: Some(id(2)),
            },
            ZoneCommand::SetTarget {
                entity: id(1),
                target: None,
            },
            ZoneCommand::Attack { entity: id(1) },
            ZoneCommand::StopAttack { entity: id(1) },
            ZoneCommand::Respawn { entity: id(1) },
        ] {
            let applied = AppliedCommand {
                ordinal: Ordinal(7),
                source: CommandSource::Session {
                    entity: id(1),
                    generation: SessionGeneration(3),
                },
                seq: Some(5),
                command,
            };
            let encoded = command_to_pb(&applied).encode_to_vec();
            assert_eq!(
                command_from_pb(PbCommand::decode(encoded.as_slice()).unwrap()).unwrap(),
                applied
            );
        }
        for reason in [
            RejectReason::DeadActor,
            RejectReason::NonAttackableTarget,
            RejectReason::TargetNotInAoi,
            RejectReason::OutOfRange,
            RejectReason::Protected,
            RejectReason::NotYetImplemented,
        ] {
            assert_eq!(reason_from_pb(reason_to_pb(reason)).unwrap(), reason);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod phase3_boundary_tests {
    use super::*;

    pub(super) fn empty(schema: u32) -> PbRecord {
        PbRecord {
            schema,
            state_digest: Bytes::from(vec![0; 32]),
            ..PbRecord::default()
        }
    }
    pub(super) fn nested(tag: u32, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        prost::encoding::encode_key(tag, prost::encoding::WireType::LengthDelimited, &mut bytes);
        prost::encoding::encode_varint(value.len() as u64, &mut bytes);
        bytes.extend_from_slice(value);
        bytes
    }
    pub(super) fn scalar(tag: u32, value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        prost::encoding::encode_key(tag, prost::encoding::WireType::Varint, &mut bytes);
        prost::encoding::encode_varint(value, &mut bytes);
        bytes
    }

    #[test]
    fn all_record_versions_remain_readable_and_byte_exact() {
        for (schema, digest) in [
            (3, StateDigestVersion::JsonV1),
            (4, StateDigestVersion::BinaryV2),
            (5, StateDigestVersion::BinaryV3),
            (6, StateDigestVersion::BinaryV4),
            (7, StateDigestVersion::BinaryV5),
            (8, StateDigestVersion::BinaryV6),
            (9, StateDigestVersion::BinaryV7),
        ] {
            let bytes = empty(schema).encode_to_vec();
            let record = decode_record(&bytes).unwrap();
            assert_eq!(record.digest_version, digest);
            assert_eq!(record.try_encode().unwrap(), bytes);
        }
    }

    #[test]
    fn six_v5_carriers_roundtrip_only_through_record_seven_slots() {
        use crate::domain::zone::{
            AttackResultPhase3, AttackStartedPhase3, CombatViewPhase3, OfferId, PvpState,
            ResurrectOfferState, TargetChangedPhase3,
        };
        let id = crate::domain::zone::EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let binding = crate::domain::zone::effects::TargetBinding {
            entity: id,
            incarnation: 2,
        };
        let events = vec![
            ZoneEvent::AttackResult {
                tick: Tick(1),
                attacker: id,
                target: id,
                target_incarnation: 2,
                outcome: crate::domain::zone::AttackOutcome::Hit,
                damage: 1,
                target_hp_after: 4,
                phase3: Some(AttackResultPhase3 {
                    attacker_incarnation: 2,
                    cp_absorbed: 7,
                }),
            },
            ZoneEvent::AttackStarted {
                tick: Tick(1),
                attacker: id,
                target: id,
                target_incarnation: 2,
                impact: Tick(3),
                ready: Tick(5),
                phase3: Some(AttackStartedPhase3 {
                    attacker_incarnation: 2,
                    target_visible: false,
                    baseline: Some(Tick(2)),
                }),
            },
            ZoneEvent::TargetChanged {
                tick: Tick(1),
                entity: id,
                target: None,
                phase3: Some(TargetChangedPhase3 {
                    target_incarnation: None,
                    baseline: Some(Tick(1)),
                }),
            },
            ZoneEvent::EntitySpawn {
                phase4: None,
                phase5: None,
                tick: Tick(1),
                entity: id,
                kind: EntityKind::Player,
                name: "Visible".into(),
                identity: None,
                pos: Vec2Fixed::from_tiles(0, 0),
                dest: None,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                combat: Some(CombatView {
                    phase3: Some(CombatViewPhase3 {
                        status_flags: crate::domain::zone::effects::StatusFlags::STUN,
                        flag_until: Some(Tick(1200)),
                        karma: 5,
                    }),
                    template: None,
                    incarnation: 2,
                    dead: false,
                    attackable: true,
                    hp: 4,
                    max_hp: 10,
                    level: 1,
                    swing: None,
                }),
            },
            ZoneEvent::PvpStateChanged {
                tick: Tick(1),
                entity: id,
                incarnation: 2,
                pvp: PvpState {
                    flag_until: None,
                    karma: 0,
                    pk_count: 0,
                    pvp_count: 0,
                },
                baseline: None,
            },
            ZoneEvent::ResurrectOffer {
                tick: Tick(1),
                target: binding,
                offer_id: OfferId(1),
                caster: None,
                restore_percent_q: 20_000_000,
                recoverable_xp: 100,
                expires_at: Tick(601),
                state: ResurrectOfferState::Offered,
            },
        ];
        for event in events {
            assert_eq!(event_from_pb(event_to_pb(&event)).unwrap(), event);
            let mut record = empty(7);
            record.events = PbOutputs {
                items: vec![PbOutput {
                    item: Some(event_to_pb(&event)),
                }],
            }
            .encode_to_vec()
            .into();
            assert!(decode_record(&record.encode_to_vec()).is_ok());
            for schema in 3..=6 {
                record.schema = schema;
                assert!(decode_record(&record.encode_to_vec()).is_err());
            }
        }
    }

    #[test]
    fn old_records_reject_phase3_presence_at_every_protobuf_depth() {
        for schema in 3..=6 {
            // Empty/default extensions are still present, and prost would discard them.
            for injection in [
                nested(5, &nested(16, &[])),                         // CastSkill
                nested(5, &nested(4, &nested(6, &nested(9, &[])))),  // PlayerLoad.phase3
                nested(5, &nested(5, &nested(4, &nested(6, &[])))),  // NpcCombat.phase3
                nested(5, &nested(7, &nested(3, &nested(9, &[])))),  // forbidden replacement load
                nested(9, &nested(1, &nested(26, &[]))),             // PvP state
                nested(7, &nested(2, &nested(1, &nested(27, &[])))), // owner offer
                nested(9, &nested(1, &nested(20, &[]))),             // skill event
                nested(9, &nested(1, &nested(1, &nested(9, &nested(9, &[]))))), // CombatView
                nested(7, &nested(2, &nested(1, &nested(31, &[])))), // observer projection
                nested(6, &scalar(5, 21)),                           // new rejection
                nested(7, &nested(2, &nested(1, &nested(4, &scalar(5, 21))))),
            ] {
                let mut bytes = empty(schema).encode_to_vec();
                bytes.extend(injection);
                let error = decode_record(&bytes).unwrap_err();
                assert!(error.0.contains("legacy"), "{error}");
            }
        }
    }

    #[test]
    fn old_json_tags_cannot_smuggle_extensions_in_nested_receipts_or_projections() {
        for schema in 5..=6 {
            for json in [
                br#"{"phase3":null}"#.as_slice(),
                br#"{"class_state":{"successful_transfer_receipts":[{"result":{"phase3":{}}}]}}"#,
                br#"{"combat":{"progression":{"phase3":{}}}}"#,
                br#"{"checkpoint":{"outbox":[{"karma":0}]}}"#,
            ] {
                for injection in [
                    nested(5, &nested(4, &nested(6, &nested(8, json)))),
                    nested(5, &nested(15, json)),
                    nested(9, &nested(1, &nested(18, json))),
                    nested(7, &nested(2, &nested(1, &nested(18, json)))),
                ] {
                    let mut bytes = empty(schema).encode_to_vec();
                    bytes.extend(injection);
                    let error = decode_record(&bytes).unwrap_err();
                    assert!(error.0.contains("Phase 3 field"), "{error}");
                }
            }
        }
    }

    #[test]
    fn legacy_record_json_payloads_reject_nested_learning_outbox() {
        let json = br#"{"checkpoints":[{"events":[{"type":"character_skill_learned"}]}]}"#;
        for schema in 5..=6 {
            for injection in [
                nested(5, &nested(4, &nested(6, &nested(8, json)))),
                nested(5, &nested(15, json)),
                nested(9, &nested(1, &nested(18, json))),
                nested(7, &nested(2, &nested(1, &nested(18, json)))),
            ] {
                let mut bytes = empty(schema).encode_to_vec();
                bytes.extend(injection);
                let error = decode_record(&bytes).unwrap_err();
                assert!(error.0.contains("Phase 3 event character_skill_learned"), "{error}");
            }
        }
    }

    #[test]
    fn phase3_event_slots_reject_wrong_variants_and_unknown_nested_extensions() {
        let event = ZoneEvent::SkillCastResult {
            tick: Tick(4),
            caster: crate::domain::zone::effects::TargetBinding {
                entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
                incarnation: 0,
            },
            cast_id: crate::domain::zone::CastId(2),
            skill: crate::domain::zone::skill::SkillRef {
                id: crate::domain::zone::skill::SkillId(1177),
                rank: 1,
            },
            targets: Vec::new(),
        };
        let bytes = canonical_json(&event);
        assert_eq!(event_from_pb(PbOutputItem::SkillCastResult(bytes.clone())).unwrap(), event);
        assert!(event_from_pb(PbOutputItem::SkillCastStarted(bytes.clone())).is_err());
        assert!(event_from_pb(PbOutputItem::Phase3Projection(bytes.clone())).is_err());
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let caster_json = json.get_mut("caster").unwrap();
        caster_json
            .as_object_mut()
            .unwrap()
            .insert("future_extension".into(), serde_json::Value::Null);
        assert!(event_from_pb(PbOutputItem::SkillCastResult(serde_json::to_vec(&json).unwrap()))
            .is_err());
        for schema in 3..=6 {
            let mut record = empty(schema).encode_to_vec();
            record.extend(nested(9, &nested(1, &nested(21, &bytes))));
            assert!(decode_record(&record).is_err());
        }
    }

    #[test]
    fn digest_comparison_validates_full_reproduction_before_hashing() {
        let mut invalid = decode_record(&empty(6).encode_to_vec()).unwrap();
        invalid.events = Bytes::from(nested(1, &nested(18, br#"{"phase3":{}}"#)));
        let hashed = invalid.with_output_digests();
        assert!(hashed.validate().is_ok()); // A digest has no inspectable event fields.
        assert!(!hashed.reproduced_by(&invalid));
        assert!(invalid.try_encode().is_err());
    }
}

#[cfg(test)]
#[path = "codec_phase5_tests.rs"]
mod phase5_boundary_tests;
