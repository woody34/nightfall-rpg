//! Domain events: facts about something that happened. Published on the event bus after the
//! transaction that produced them commits (docs/engineering/architecture.md §3).

use super::{AccountId, CharacterId, Race};
use serde::{Deserialize, Serialize};

/// Every event the domain can emit. Serialized as JSON on the bus with a `type` tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
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
}

impl DomainEvent {
    /// NATS subject for the event. Shape: `nightfall.<aggregate>.<event>`.
    #[must_use]
    pub const fn subject(&self) -> &'static str {
        match self {
            DomainEvent::CharacterCreated { .. } => "nightfall.character.created",
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
    }
}
