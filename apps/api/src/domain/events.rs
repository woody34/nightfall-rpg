//! Domain events: facts about something that happened. Published on the event bus after the
//! transaction that produced them commits (docs/engineering/architecture.md §3).

use super::class::ClassId;
use super::{AccountId, CharacterId, Race};
use serde::{Deserialize, Serialize};

/// Every event the domain can emit. Serialized as JSON on the bus with a `type` tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
#[allow(clippy::enum_variant_names)]
pub enum DomainEvent {
    /// A character was created.
    CharacterCreated {
        /// The new character.
        character_id: CharacterId,
        /// Owning account.
        account_id: AccountId,
        /// Race chosen.
        race: Race,
    },
    /// A character reached a new level (one event per level gained).
    CharacterLeveled {
        /// Stable identity and aggregate order of this fact.
        metadata: EventMetadata,
        /// The character.
        character_id: CharacterId,
        /// The level reached.
        level: u32,
    },
    /// A successful, durably checkpointed class transfer.
    CharacterClassChanged {
        /// Stable event identity and aggregate sequence.
        metadata: EventMetadata,
        /// The character.
        character_id: CharacterId,
        /// Profession before transfer.
        old_class_id: ClassId,
        /// Profession after transfer.
        new_class_id: ClassId,
        /// Deterministic transfer tick.
        tick: u64,
        /// Client mutation UUID.
        request_key: uuid::Uuid,
    },
    /// A character died.
    CharacterDied {
        /// Stable identity and aggregate order of this fact.
        metadata: EventMetadata,
        /// Level after the death penalty.
        level: u32,
        /// Cumulative XP after the penalty.
        xp: u64,
        /// XP charged by this death.
        xp_lost: u64,
        /// The character.
        character_id: CharacterId,
        /// What dealt the killing blow, as an opaque id (monster template or character id).
        killer: String,
    },
}

/// Deterministic progression event identity. Sequence orders checkpoints, then facts within
/// a checkpoint; it remains ordered across reconnects and epochs through the repository fence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMetadata {
    /// Hash of character, zone, epoch, tick and fact ordinal.
    pub event_id: uuid::Uuid,
    /// Lexicographic aggregate sequence: committed revision, then event ordinal.
    pub sequence: (u64, u64),
}

impl DomainEvent {
    /// NATS subject for the event. Shape: `nightfall.<aggregate>.<event>`.
    #[must_use]
    pub const fn subject(&self) -> &'static str {
        match self {
            DomainEvent::CharacterCreated { .. } => "nightfall.character.created",
            DomainEvent::CharacterLeveled { .. } => "nightfall.character.leveled",
            DomainEvent::CharacterDied { .. } => "nightfall.character.died",
            DomainEvent::CharacterClassChanged { .. } => "nightfall.character.class_changed",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn serializes_with_type_tag() {
        let ev = DomainEvent::CharacterCreated {
            character_id: CharacterId::from_uuid(Uuid::nil()),
            account_id: AccountId::from_uuid(Uuid::nil()),
            race: Race::Elf,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "character_created");
        assert_eq!(json["race"], "elf");
        let back: DomainEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn subject_follows_naming_convention() {
        let ev = DomainEvent::CharacterCreated {
            character_id: CharacterId::new(),
            account_id: AccountId::from_uuid(Uuid::nil()),
            race: Race::Human,
        };
        assert_eq!(ev.subject(), "nightfall.character.created");
        let id = CharacterId::new();
        assert_eq!(
            DomainEvent::CharacterLeveled {
                metadata: EventMetadata::default(),
                character_id: id,
                level: 2
            }
            .subject(),
            "nightfall.character.leveled"
        );
        let died = DomainEvent::CharacterDied {
            metadata: EventMetadata::default(),
            level: 1,
            xp: 0,
            xp_lost: 0,
            character_id: id,
            killer: "npc:1".into(),
        };
        assert_eq!(died.subject(), "nightfall.character.died");
        let json = serde_json::to_value(&died).unwrap();
        assert_eq!(json["type"], "character_died");
        assert_eq!(serde_json::from_value::<DomainEvent>(json).unwrap(), died);
    }
}
