//! Story 2.2: every Phase 5 kind/tag round-trips only through Record 9, and Record ≤ 8
//! refuses every Phase 5 carrier recursively, including present-but-empty fields.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::phase3_boundary_tests::{empty, nested, scalar};
use super::*;
use crate::domain::zone::phase5_command::samples as command_samples;
use crate::domain::zone::phase5_event::samples as event_samples;
use crate::domain::zone::phase5_wire::{
    GROUND_ITEM_ENTITY_KIND, PHASE5_FIRST_REJECT_REASON, PHASE5_PLAYER_LOAD_TAG,
};

fn record_with(schema: u32, commands: Vec<PbCommand>, events: &[ZoneEvent]) -> Vec<u8> {
    let mut record = empty(schema);
    record.commands = commands;
    record.events = encode_events(events).into();
    record.encode_to_vec()
}

#[allow(clippy::wildcard_enum_match_arm)] // Only the Phase 5 source rule matters here.
fn applied(command: ZoneCommand) -> AppliedCommand {
    let source = match &command {
        ZoneCommand::Phase5(c) if c.kind().is_system() => CommandSource::System,
        ZoneCommand::Phase5(c) => CommandSource::Session {
            entity: c.entity().unwrap(),
            generation: SessionGeneration(1),
        },
        _ => CommandSource::System,
    };
    AppliedCommand {
        ordinal: Ordinal(1),
        source,
        seq: None,
        command,
    }
}

#[test]
fn every_command_kind_round_trips_through_record_nine_only() {
    let samples = command_samples::all();
    assert_eq!(samples.len(), 45);
    for (command, kind) in samples.into_iter().zip(Phase5CommandKind::ALL) {
        let applied = applied(ZoneCommand::Phase5(Box::new(command.clone())));
        let pb = command_to_pb(&applied);
        // The oneof tag is the design §12.2 number: re-encode and read the key.
        let mut key = Vec::new();
        prost::encoding::encode_key(
            kind.codec_tag(),
            prost::encoding::WireType::LengthDelimited,
            &mut key,
        );
        let encoded = pb.encode_to_vec();
        assert!(
            encoded.windows(key.len()).any(|w| w == key.as_slice()),
            "{kind:?} is not carried on tag {}",
            kind.codec_tag()
        );
        assert_eq!(command_from_pb(pb.clone()).unwrap(), applied, "{kind:?}");
        for schema in 3..=9 {
            let decoded = decode_record(&record_with(schema, vec![pb.clone()], &[]));
            assert_eq!(decoded.is_ok(), schema == 9, "{kind:?} under record {schema}");
            if schema < 9 {
                assert!(decoded.unwrap_err().0.contains("Phase 5"), "{kind:?}");
            }
        }
        let record = decode_record(&record_with(9, vec![pb], &[])).unwrap();
        assert_eq!(record.digest_version, StateDigestVersion::BinaryV7);
        assert_eq!(record.commands, vec![applied]);
        assert_eq!(
            record.try_encode().unwrap(),
            record_with(9, record_to_pb(&record).commands, &[])
        );
    }
}

#[test]
fn command_bodies_require_canonical_bytes_matching_kinds_and_valid_structure() {
    let command = command_samples::all()[0].clone();
    let bytes = canonical_json(&command);
    assert!(phase5::command_from_json(&bytes, Phase5CommandKind::PickUpItem).is_ok());
    assert!(phase5::command_from_json(&bytes, Phase5CommandKind::VendorBuy).is_err());
    let mut whitespace = bytes.clone();
    whitespace.push(b' ');
    assert!(phase5::command_from_json(&whitespace, Phase5CommandKind::PickUpItem).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["request_key"] = serde_json::Value::String(uuid::Uuid::nil().to_string());
    let nil_key = serde_json::to_vec(&value).unwrap();
    assert!(phase5::command_from_json(&nil_key, Phase5CommandKind::PickUpItem).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["frozen"] = serde_json::json!({});
    assert!(phase5::command_from_json(
        &serde_json::to_vec(&value).unwrap(),
        Phase5CommandKind::PickUpItem
    )
    .is_err());
    // A mislabelled carrier is refused by the raw grammar before permissive decoding.
    let record = record_with(
        9,
        vec![PbCommand {
            ordinal: 1,
            session: None,
            seq: None,
            kind: Some(PbCommandKind::VendorBuy(bytes)),
        }],
        &[],
    );
    assert!(decode_record(&record).is_err());
}

#[test]
fn every_output_carrier_round_trips_through_record_nine_only() {
    let mut events: Vec<(Phase5OutputKind, ZoneEvent)> = Vec::new();
    events.push((
        Phase5OutputKind::Phase5Enriched,
        ZoneEvent::EntitySpawn {
            identity: None,
            phase4: None,
            phase5: Some(Box::new(event_samples::ground_item())),
            tick: Tick(40),
            entity: EntityId::from_uuid(uuid::Uuid::from_u128(9)),
            kind: EntityKind::Npc,
            name: String::new(),
            pos: Vec2Fixed::from_tiles(3, 4),
            dest: None,
            speed: Speed::DEFAULT,
            generation: SessionGeneration::default(),
            combat: None,
        },
    ));
    for event in event_samples::events() {
        events.push((event.body.output_kind(), ZoneEvent::Economy(Box::new(event))));
    }
    events.push((
        Phase5OutputKind::EconomyCheckpoint,
        ZoneEvent::EconomyCheckpoint(Box::new(event_samples::checkpoint())),
    ));
    assert_eq!(events.len(), Phase5OutputKind::ALL.len());
    for ((kind, event), expected) in events.iter().zip(Phase5OutputKind::ALL) {
        assert_eq!(*kind, expected);
        let pb = event_to_pb(event);
        let mut key = Vec::new();
        prost::encoding::encode_key(
            kind.codec_tag(),
            prost::encoding::WireType::LengthDelimited,
            &mut key,
        );
        let encoded = PbOutput {
            item: Some(pb.clone()),
        }
        .encode_to_vec();
        assert!(encoded.starts_with(&key), "{kind:?} is not carried on tag {}", kind.codec_tag());
        assert_eq!(&event_from_pb(pb).unwrap(), event, "{kind:?}");
        for schema in 3..=9 {
            let decoded =
                decode_record(&record_with(schema, Vec::new(), std::slice::from_ref(event)));
            assert_eq!(decoded.is_ok(), schema == 9, "{kind:?} under record {schema}");
        }
        assert_eq!(
            decode_events(&encode_events(std::slice::from_ref(event))).unwrap(),
            vec![event.clone()]
        );
    }
}

#[test]
fn reserved_output_51_is_refused_under_every_record_schema() {
    for schema in 3..=9 {
        let mut bytes = empty(schema).encode_to_vec();
        bytes.extend(nested(9, &nested(1, &nested(51, b"{}"))));
        let error = decode_record(&bytes).unwrap_err();
        assert!(error.0.contains("reserved"), "{error}");
    }
}

#[test]
fn player_load_tag_eleven_round_trips_only_under_record_nine() {
    let mut load = PlayerLoad::fresh("fighter");
    load.phase5 = Some(event_samples::player_load());
    // A V7 load needs the V5 and V6 blocks (design §1.11); a lone phase5 is malformed.
    let lone = applied(ZoneCommand::SpawnPlayer {
        entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
        name: "P".into(),
        pos: Vec2Fixed::from_tiles(1, 1),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(load.clone())),
    });
    assert!(command_from_pb(command_to_pb(&lone)).is_err());
    let pb = command_to_pb(&lone).encode_to_vec();
    let mut key = Vec::new();
    prost::encoding::encode_key(
        PHASE5_PLAYER_LOAD_TAG,
        prost::encoding::WireType::LengthDelimited,
        &mut key,
    );
    assert!(pb.windows(key.len()).any(|w| w == key.as_slice()));
    for schema in 3..=8 {
        // Present but empty: prost would discard it, the raw grammar refuses it.
        let mut bytes = empty(schema).encode_to_vec();
        bytes.extend(nested(5, &nested(4, &nested(6, &nested(PHASE5_PLAYER_LOAD_TAG, &[])))));
        let error = decode_record(&bytes).unwrap_err();
        assert!(error.0.contains("Phase 5"), "{error}");
    }
    let bytes = canonical_json(&event_samples::player_load());
    assert_eq!(phase5::load_from_json(&bytes).unwrap(), event_samples::player_load());
    let mut record = empty(9).encode_to_vec();
    record.extend(nested(
        5,
        &nested(4, &nested(6, &nested(PHASE5_PLAYER_LOAD_TAG, br#"{"schema_version":2}"#))),
    ));
    assert!(decode_record(&record).is_err());
}

#[test]
fn old_records_reject_phase5_presence_at_every_protobuf_depth() {
    for schema in 3..=8 {
        for (injection, hint) in [
            (nested(5, &nested(32, &[])), "Phase 5"), // PickUpItem, empty
            (nested(5, &nested(76, &[])), "Phase 5"), // MailExpire, empty
            (nested(9, &nested(1, &nested(42, &[]))), "Phase 5"), // enriched ground spawn
            (nested(9, &nested(1, &nested(55, &[]))), "Phase 5"), // economy checkpoint
            (nested(7, &nested(2, &nested(1, &nested(43, &[])))), "Phase 5"), // owner receipt
            (nested(6, &scalar(5, PHASE5_FIRST_REJECT_REASON as u64)), "Phase 5 rejection"),
            (nested(6, &scalar(5, 102)), "Phase 5 rejection"),
            (
                nested(9, &nested(1, &nested(1, &scalar(3, GROUND_ITEM_ENTITY_KIND as u64)))),
                "Phase 5 entity kind",
            ),
        ] {
            let mut bytes = empty(schema).encode_to_vec();
            bytes.extend(&injection);
            let error = decode_record(&bytes).unwrap_err();
            assert!(error.0.contains(hint), "record {schema} ({injection:?}): {error}");
        }
    }
    // Record 9 admits the ground-item kind value at the grammar; the enum variant lands with
    // the proto (story 2.1), so whatever the permissive decoder says, it is not a Phase 5 refusal.
    let mut bytes = empty(9).encode_to_vec();
    bytes.extend(nested(9, &nested(1, &nested(1, &scalar(3, 3)))));
    if let Err(error) = decode_record(&bytes) {
        assert!(!error.0.contains("Phase 5"), "{error}");
    }
}

#[test]
fn old_json_carriers_cannot_smuggle_v7_names() {
    for json in [
        br#"{"phase5":null}"#.as_slice(),
        br#"{"checkpoint":{"inventory":{"7":{"location":"warehouse"}}}}"#,
        br#"{"deltas":[{"operation":"transfer_out"}]}"#,
        br#"{"deltas":[{"op":"loot"}]}"#,
        br#"{"receipt":{"operation":"pick_up_item"}}"#,
        br#"{"type":"economy"}"#,
        br#"{"type":"economy_checkpoint"}"#,
        br#"{"spawn":{"kind":"ground_item"}}"#,
        br#"{"lease_releases":[]}"#,
        br#"{"nested":[{"ground_item":{}}]}"#,
    ] {
        let value: serde_json::Value = serde_json::from_slice(json).unwrap();
        assert!(super::super::codec_validation::pre_phase5_json(&value).is_err(), "{value}");
        for schema in 5..=8 {
            for injection in [
                nested(5, &nested(15, json)),             // ChangeClass
                nested(9, &nested(1, &nested(18, json))), // Phase2 event
                nested(7, &nested(2, &nested(1, &nested(18, json)))),
                nested(5, &nested(4, &nested(6, &nested(8, json)))), // progression load
            ] {
                let mut bytes = empty(schema).encode_to_vec();
                bytes.extend(injection);
                assert!(decode_record(&bytes).is_err(), "record {schema}: {value}");
            }
        }
        {
            let schema = 8;
            for injection in [
                nested(5, &nested(4, &nested(6, &nested(10, json)))), // Phase4 load
                nested(9, &nested(1, &nested(40, json))),             // ItemCheckpoint
                nested(5, &nested(20, json)),                         // EquipItem
            ] {
                let mut bytes = empty(schema).encode_to_vec();
                bytes.extend(injection);
                let error = decode_record(&bytes).unwrap_err();
                assert!(error.0.contains("V7"), "record 8: {error}");
            }
        }
    }
    // Historical V6 names stay admitted by the scan itself.
    let ok: serde_json::Value = serde_json::json!({
        "location": "paperdoll", "operation": "consume", "kind": "player", "type": "entity_spawn"
    });
    super::super::codec_validation::pre_phase5_json(&ok).unwrap();
}

#[test]
fn older_carriers_refuse_a_ground_item_enrichment() {
    let spawn = ZoneEvent::EntitySpawn {
        identity: None,
        phase4: None,
        phase5: Some(Box::new(event_samples::ground_item())),
        tick: Tick(40),
        entity: EntityId::from_uuid(uuid::Uuid::from_u128(9)),
        kind: EntityKind::Npc,
        name: String::new(),
        pos: Vec2Fixed::from_tiles(3, 4),
        dest: None,
        speed: Speed::DEFAULT,
        generation: SessionGeneration::default(),
        combat: None,
    };
    let bytes = canonical_json(&spawn);
    assert!(event_from_pb(PbOutputItem::Phase4Enriched(bytes.clone())).is_err());
    assert!(event_from_pb(PbOutputItem::Phase3Projection(bytes.clone())).is_err());
    assert!(event_from_pb(PbOutputItem::Phase2(bytes.clone())).is_err());
    assert_eq!(event_from_pb(PbOutputItem::Phase5Enriched(bytes.clone())).unwrap(), spawn);
    // Tag 42 carries ground items only: a player identity or combat view is refused.
    let mut player = spawn.clone();
    if let ZoneEvent::EntitySpawn { combat, .. } = &mut player {
        *combat = Some(CombatView {
            phase3: None,
            template: None,
            incarnation: 1,
            dead: false,
            attackable: true,
            hp: 1,
            max_hp: 1,
            level: 1,
            swing: None,
        });
    }
    assert!(event_from_pb(PbOutputItem::Phase5Enriched(canonical_json(&player))).is_err());
    let mut zero = spawn;
    if let ZoneEvent::EntitySpawn {
        phase5: Some(detail),
        ..
    } = &mut zero
    {
        detail.count = 0;
    }
    assert!(event_from_pb(PbOutputItem::Phase5Enriched(canonical_json(&zero))).is_err());
}

#[test]
fn record_schema_nine_is_binary_v7_and_older_schemas_stay_byte_exact() {
    let record = decode_record(&empty(9).encode_to_vec()).unwrap();
    assert_eq!(record.digest_version, StateDigestVersion::BinaryV7);
    assert_eq!(record.try_encode().unwrap(), empty(9).encode_to_vec());
    assert_eq!(RECORD_SCHEMA_VERSION, 9);
    let v6 = decode_record(&empty(8).encode_to_vec()).unwrap();
    assert_eq!(v6.digest_version, StateDigestVersion::BinaryV6);
    assert_eq!(v6.try_encode().unwrap(), empty(8).encode_to_vec());
}
