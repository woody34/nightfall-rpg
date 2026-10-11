//! Record7 command payloads. These tag numbers are independent of the public wire.

use super::{entity_from, uuid_from, CodecError, Message, ZoneCommand};
use crate::domain::zone::{skill::SkillId, CastId, OfferId};

#[derive(Clone, PartialEq, Message)]
pub(super) struct PbCastSkill {
    #[prost(bytes = "vec", tag = "1")]
    pub entity: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub skill_id: u32,
    #[prost(bytes = "vec", optional, tag = "3")]
    pub target: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct PbCancelCast {
    #[prost(bytes = "vec", tag = "1")]
    pub entity: Vec<u8>,
    #[prost(uint64, optional, tag = "2")]
    pub cast_id: Option<u64>,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct PbAcceptResurrect {
    #[prost(bytes = "vec", tag = "1")]
    pub entity: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub offer_id: u64,
    #[prost(bool, tag = "3")]
    pub accept: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct PbLearnSkill {
    #[prost(bytes = "vec", tag = "1")]
    pub entity: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub account: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub request_key: Vec<u8>,
    #[prost(uint32, tag = "4")]
    pub skill_id: u32,
    #[prost(uint32, tag = "5")]
    pub skill_level: u32,
}

fn positive(value: u64, field: &str) -> Result<(), CodecError> {
    if value == 0 {
        Err(CodecError(format!("Phase3 {field} must be positive")))
    } else {
        Ok(())
    }
}

impl PbCastSkill {
    pub(super) fn into_command(self) -> Result<ZoneCommand, CodecError> {
        positive(u64::from(self.skill_id), "skill_id")?;
        Ok(ZoneCommand::CastSkill {
            entity: entity_from(&self.entity)?,
            skill_id: SkillId(self.skill_id),
            target: self.target.as_deref().map(entity_from).transpose()?,
        })
    }
}
impl PbCancelCast {
    pub(super) fn into_command(self) -> Result<ZoneCommand, CodecError> {
        if let Some(id) = self.cast_id {
            positive(id, "cast_id")?;
        }
        Ok(ZoneCommand::CancelCast {
            entity: entity_from(&self.entity)?,
            cast_id: self.cast_id.map(CastId),
        })
    }
}
impl PbAcceptResurrect {
    pub(super) fn into_command(self) -> Result<ZoneCommand, CodecError> {
        positive(self.offer_id, "offer_id")?;
        Ok(ZoneCommand::AcceptResurrect {
            entity: entity_from(&self.entity)?,
            offer_id: OfferId(self.offer_id),
            accept: self.accept,
        })
    }
}
impl PbLearnSkill {
    pub(super) fn into_command(self) -> Result<ZoneCommand, CodecError> {
        positive(u64::from(self.skill_id), "skill_id")?;
        positive(u64::from(self.skill_level), "skill_level")?;
        Ok(ZoneCommand::LearnSkill {
            entity: entity_from(&self.entity)?,
            account: crate::domain::AccountId::from_uuid(uuid_from(&self.account)?),
            request_key: uuid_from(&self.request_key)?,
            skill_id: SkillId(self.skill_id),
            skill_level: self.skill_level,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::*;
    use super::*;

    fn applied(command: ZoneCommand) -> AppliedCommand {
        AppliedCommand {
            ordinal: Ordinal(u64::MAX),
            source: CommandSource::Session {
                entity: EntityId::from_uuid(Uuid::from_u128(1)),
                generation: SessionGeneration(u64::MAX),
            },
            seq: Some(u32::MAX),
            command,
        }
    }

    #[test]
    fn learning_fact_slot30_rejects_legacy_nested_injection_and_inconsistent_identity() {
        use crate::domain::zone::{FrozenLearnResult, LearnedEffectiveStats};
        let entity = EntityId::from_uuid(Uuid::from_u128(1));
        let event = ZoneEvent::SkillLearned {
            tick: Tick(5),
            entity,
            account: crate::domain::AccountId::from_uuid(Uuid::from_u128(2)),
            request_key: Uuid::from_u128(3),
            result: Box::new(FrozenLearnResult {
                character_id: crate::domain::CharacterId::from_uuid(entity.as_uuid()),
                skill: crate::domain::zone::skill::SkillRef {
                    id: SkillId(3),
                    rank: 1,
                },
                sp_spent: 50,
                sp_remaining: 100,
                tick: Tick(5),
                hp: 1,
                max_hp: 10,
                mp: 2,
                max_mp: 20,
                cp: 3,
                max_cp: 30,
                effective_stats: LearnedEffectiveStats::default(),
                incarnation: 1,
            }),
        };
        let encoded = event_to_pb(&event);
        assert!(matches!(encoded, PbOutputItem::SkillLearned(_)));
        assert_eq!(event_from_pb(encoded.clone()).unwrap(), event);
        let payload = PbOutputs {
            items: vec![PbOutput {
                item: Some(encoded),
            }],
        }
        .encode_to_vec();
        for schema in 3..=7 {
            for as_observer_output in [false, true] {
                let mut record = PbRecord {
                    schema,
                    state_digest: Bytes::from(vec![0; 32]),
                    ..PbRecord::default()
                };
                if as_observer_output {
                    record.outputs.push(PbPlayerOutput {
                        entity: entity.as_uuid().as_bytes().to_vec(),
                        encoded: Bytes::from(payload.clone()),
                        ..PbPlayerOutput::default()
                    });
                } else {
                    record.events = Bytes::from(payload.clone());
                }
                assert_eq!(decode_record(&record.encode_to_vec()).is_ok(), schema == 7);
            }
        }
        let mut value = serde_json::to_value(&event).unwrap();
        value
            .get_mut("result")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown_nested_extension".into(), serde_json::json!(0));
        assert!(
            event_from_pb(PbOutputItem::SkillLearned(serde_json::to_vec(&value).unwrap())).is_err()
        );
        let mut inconsistent = event;
        if let ZoneEvent::SkillLearned { result, .. } = &mut inconsistent {
            result.tick = Tick(6);
        }
        assert!(event_from_pb(event_to_pb(&inconsistent)).is_err());
    }

    #[test]
    fn learning_refusals_have_additive_codes_and_are_forbidden_in_legacy_records() {
        for (code, reason) in [
            (33, RejectReason::SkillNotLearnable),
            (34, RejectReason::SkillWrongRank),
            (35, RejectReason::SkillLevelRequired),
            (36, RejectReason::SkillItemRequired),
            (37, RejectReason::SkillDeferred),
            (38, RejectReason::InsufficientSp),
        ] {
            assert_eq!(reason_to_pb(reason), code);
            assert_eq!(reason_from_pb(code).unwrap(), reason);
            for schema in 3..=7 {
                let pb = PbRecord {
                    schema,
                    state_digest: Bytes::from(vec![0; 32]),
                    dispositions: vec![PbDisposition {
                        reason: code,
                        ..PbDisposition::default()
                    }],
                    ..PbRecord::default()
                };
                assert_eq!(decode_record(&pb.encode_to_vec()).is_ok(), schema == 7);
            }
        }
    }

    #[test]
    fn commands_have_distinct_pinned_tags_and_preserve_optional_identities() {
        let entity = EntityId::from_uuid(Uuid::from_u128(1));
        let target = EntityId::from_uuid(Uuid::from_u128(2));
        let commands = [
            (
                16,
                ZoneCommand::CastSkill {
                    entity,
                    skill_id: SkillId(1177),
                    target: Some(target),
                },
            ),
            (
                16,
                ZoneCommand::CastSkill {
                    entity,
                    skill_id: SkillId(3),
                    target: None,
                },
            ),
            (
                17,
                ZoneCommand::CancelCast {
                    entity,
                    cast_id: Some(CastId(u64::MAX)),
                },
            ),
            (
                17,
                ZoneCommand::CancelCast {
                    entity,
                    cast_id: None,
                },
            ),
            (
                18,
                ZoneCommand::AcceptResurrect {
                    entity,
                    offer_id: OfferId(u64::MAX),
                    accept: true,
                },
            ),
            (
                18,
                ZoneCommand::AcceptResurrect {
                    entity,
                    offer_id: OfferId(1),
                    accept: false,
                },
            ),
            (
                19,
                ZoneCommand::LearnSkill {
                    entity,
                    account: crate::domain::AccountId::from_uuid(Uuid::from_u128(3)),
                    request_key: Uuid::from_u128(4),
                    skill_id: SkillId(3),
                    skill_level: 2,
                },
            ),
        ];
        for (tag, command) in commands {
            let command = applied(command);
            let pb = command_to_pb(&command);
            let mut key = Vec::new();
            prost::encoding::encode_key(tag, prost::encoding::WireType::LengthDelimited, &mut key);
            let kind_bytes = PbCommand {
                ordinal: 0,
                session: None,
                seq: None,
                kind: pb.kind.clone(),
            }
            .encode_to_vec();
            assert!(kind_bytes.starts_with(&key));
            assert_eq!(command_from_pb(pb.clone()).unwrap(), command);
            for schema in 3..=7 {
                let record = PbRecord {
                    schema,
                    commands: vec![pb.clone()],
                    state_digest: Bytes::from(vec![0; 32]),
                    ..PbRecord::default()
                };
                let bytes = record.encode_to_vec();
                if schema < 7 {
                    assert!(decode_record(&bytes).is_err());
                } else {
                    let decoded = decode_record(&bytes).unwrap();
                    assert_eq!(decoded.commands, vec![command.clone()]);
                    assert_eq!(decoded.try_encode().unwrap(), bytes);
                    assert!(command_admission_size(&crate::domain::zone::ZoneInput {
                        source: command.source,
                        seq: command.seq,
                        command: command.command.clone()
                    })
                    .is_ok());
                }
            }
        }
    }

    #[test]
    fn invalid_payloads_cannot_be_silently_normalized() {
        let entity = vec![0; 16];
        assert!(PbCastSkill {
            entity: entity.clone(),
            skill_id: 0,
            target: None
        }
        .into_command()
        .is_err());
        assert!(PbCastSkill {
            entity: entity.clone(),
            skill_id: 1,
            target: Some(vec![])
        }
        .into_command()
        .is_err());
        assert!(PbCancelCast {
            entity: entity.clone(),
            cast_id: Some(0)
        }
        .into_command()
        .is_err());
        assert!(PbAcceptResurrect {
            entity: entity.clone(),
            offer_id: 0,
            accept: true
        }
        .into_command()
        .is_err());
        assert!(PbLearnSkill {
            entity: entity.clone(),
            account: entity.clone(),
            request_key: entity.clone(),
            skill_id: 1,
            skill_level: 0
        }
        .into_command()
        .is_err());
        // prost accepts any nonzero boolean as true: raw validation must reject 2 first.
        let mut payload = PbAcceptResurrect {
            entity,
            offer_id: 1,
            accept: false,
        }
        .encode_to_vec();
        payload.extend([24, 2]);
        let mut command = Vec::new();
        prost::encoding::encode_key(18, prost::encoding::WireType::LengthDelimited, &mut command);
        prost::encoding::encode_varint(payload.len() as u64, &mut command);
        command.extend(payload);
        let mut bytes = PbRecord {
            schema: 7,
            state_digest: Bytes::from(vec![0; 32]),
            ..PbRecord::default()
        }
        .encode_to_vec();
        prost::encoding::encode_key(5, prost::encoding::WireType::LengthDelimited, &mut bytes);
        prost::encoding::encode_varint(command.len() as u64, &mut bytes);
        bytes.extend(command);
        assert!(decode_record(&bytes).unwrap_err().0.contains("boolean"));
    }
}
