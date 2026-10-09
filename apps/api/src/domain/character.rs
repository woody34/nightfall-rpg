//! Character aggregate and its value objects.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::ids::uuid_id;
use super::AccountId;

/// Playable races. Mirrors `nightfall.v1.Race` minus `UNSPECIFIED`, which is a transport
/// concern and is rejected at the interface boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Race {
    /// Balanced starting stats.
    Human,
    /// High DEX and WIT.
    Elf,
    /// High DEX and INT, low CON.
    DarkElf,
    /// High STR and CON, low INT.
    Orc,
    /// High CON, crafting and spoil.
    Dwarf,
}

impl Race {
    /// Starting base stats for the race's fighter template (Phase 1, table 2.2).
    #[must_use]
    pub const fn starting_stats(self) -> BaseStats {
        match self {
            Race::Human => BaseStats {
                str: 40,
                dex: 30,
                con: 43,
                int: 21,
                wit: 11,
                men: 25,
            },
            Race::Elf => BaseStats {
                str: 36,
                dex: 35,
                con: 36,
                int: 23,
                wit: 14,
                men: 26,
            },
            Race::DarkElf => BaseStats {
                str: 41,
                dex: 34,
                con: 32,
                int: 25,
                wit: 12,
                men: 26,
            },
            Race::Orc => BaseStats {
                str: 40,
                dex: 26,
                con: 47,
                int: 18,
                wit: 12,
                men: 27,
            },
            Race::Dwarf => BaseStats {
                str: 39,
                dex: 29,
                con: 45,
                int: 20,
                wit: 10,
                men: 27,
            },
        }
    }

    /// Id of the race's starting fighter profile (`packages/data/classes/<id>.toml`).
    #[must_use]
    pub const fn starting_class_profile(self) -> &'static str {
        match self {
            Race::Human => "human_fighter",
            Race::Elf => "elven_fighter",
            Race::DarkElf => "dark_fighter",
            Race::Orc => "orc_fighter",
            Race::Dwarf => "dwarven_fighter",
        }
    }

    /// Stable string form used in the database and in NATS subjects.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Race::Human => "human",
            Race::Elf => "elf",
            Race::DarkElf => "dark_elf",
            Race::Orc => "orc",
            Race::Dwarf => "dwarf",
        }
    }

    /// Parses the form produced by [`Race::as_str`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "human" => Some(Race::Human),
            "elf" => Some(Race::Elf),
            "dark_elf" => Some(Race::DarkElf),
            "orc" => Some(Race::Orc),
            "dwarf" => Some(Race::Dwarf),
            _ => None,
        }
    }
}

/// The six base stats. Always within `1..=MAX_STAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct BaseStats {
    pub str: u32,
    pub dex: u32,
    pub con: u32,
    pub int: u32,
    pub wit: u32,
    pub men: u32,
}

impl BaseStats {
    /// Upper bound for any single stat (Phase 1 §3: the stat tables cover `1..=99`).
    pub const MAX_STAT: u32 = 99;

    /// True when every stat is within `1..=MAX_STAT`.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let BaseStats {
            str,
            dex,
            con,
            int,
            wit,
            men,
        } = *self;
        [str, dex, con, int, wit, men]
            .iter()
            .all(|v| (1..=Self::MAX_STAT).contains(v))
    }
}

uuid_id!(
    /// Newtype over the character's UUID (v7, time-ordered, so it indexes well in Postgres).
    CharacterId
);

/// Why a name was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    /// Shorter than [`CharacterName::MIN_LEN`] or longer than [`CharacterName::MAX_LEN`].
    #[error("name must be {min}-{max} characters, got {got}")]
    Length {
        /// Minimum allowed length.
        min: usize,
        /// Maximum allowed length.
        max: usize,
        /// Length received.
        got: usize,
    },
    /// Contains something other than ASCII letters.
    #[error("name may contain ASCII letters only")]
    Charset,
}

/// A validated character name: 3 to 16 ASCII letters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CharacterName(String);

impl CharacterName {
    /// Minimum length in characters.
    pub const MIN_LEN: usize = 3;
    /// Maximum length in characters.
    pub const MAX_LEN: usize = 16;

    /// Validates and wraps a name.
    pub fn new(raw: impl Into<String>) -> Result<Self, NameError> {
        let raw = raw.into();
        let len = raw.chars().count();
        if !(Self::MIN_LEN..=Self::MAX_LEN).contains(&len) {
            return Err(NameError::Length {
                min: Self::MIN_LEN,
                max: Self::MAX_LEN,
                got: len,
            });
        }
        if !raw.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(NameError::Charset);
        }
        Ok(Self(raw))
    }

    /// Borrowed view.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Case-insensitive key for uniqueness checks.
    #[must_use]
    pub fn normalized(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl TryFrom<String> for CharacterName {
    type Error = NameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CharacterName> for String {
    fn from(value: CharacterName) -> Self {
        value.0
    }
}

impl fmt::Display for CharacterName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// World position in tile units (Phase 6 §3).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

/// The character aggregate root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Character {
    /// Stable identity.
    pub id: CharacterId,
    /// Owning account.
    pub account_id: AccountId,
    /// Unique (case-insensitive) display name.
    pub name: CharacterName,
    /// Race, fixed at creation.
    pub race: Race,
    /// Current level, `1..=MAX_LEVEL`.
    pub level: u32,
    /// Base stats before equipment and buffs.
    pub stats: BaseStats,
    /// Last known world position.
    pub position: Position,
}

impl Character {
    /// Level cap (Phase 1 §3).
    pub const MAX_LEVEL: u32 = 80;

    /// Creates a level-1 character with the race's starting stats at the origin.
    #[must_use]
    pub fn create(account_id: AccountId, name: CharacterName, race: Race) -> Self {
        Self {
            id: CharacterId::new(),
            account_id,
            name,
            race,
            level: 1,
            stats: race.starting_stats(),
            position: Position::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn race_round_trips_through_str() {
        for race in [
            Race::Human,
            Race::Elf,
            Race::DarkElf,
            Race::Orc,
            Race::Dwarf,
        ] {
            assert_eq!(Race::parse(race.as_str()), Some(race));
        }
        assert_eq!(Race::parse("kamael"), None);
    }

    #[test]
    fn starting_stats_are_valid_for_every_race() {
        for race in [
            Race::Human,
            Race::Elf,
            Race::DarkElf,
            Race::Orc,
            Race::Dwarf,
        ] {
            assert!(race.starting_stats().is_valid(), "{race:?}");
        }
    }

    #[test]
    fn name_boundaries() {
        assert!(CharacterName::new("ab").is_err(), "one below min");
        assert!(CharacterName::new("abc").is_ok(), "exactly min");
        assert!(CharacterName::new("a".repeat(16)).is_ok(), "exactly max");
        assert!(CharacterName::new("a".repeat(17)).is_err(), "one above max");
        assert_eq!(CharacterName::new("ab1").unwrap_err(), NameError::Charset);
        assert_eq!(CharacterName::new("ab c").unwrap_err(), NameError::Charset);
    }

    #[test]
    fn name_normalizes_case_insensitively() {
        let a = CharacterName::new("Aragorn").unwrap();
        let b = CharacterName::new("aRAGORN").unwrap();
        assert_eq!(a.normalized(), b.normalized());
        assert_ne!(a, b, "display form is preserved");
    }

    #[test]
    fn create_starts_at_level_one_with_race_stats() {
        let c = Character::create(
            AccountId::from_uuid(Uuid::nil()),
            CharacterName::new("Test").unwrap(),
            Race::Orc,
        );
        assert_eq!(c.level, 1);
        assert_eq!(c.stats, Race::Orc.starting_stats());
        assert_eq!(c.position, Position::default());
    }

    #[test]
    fn character_ids_are_time_ordered() {
        let a = CharacterId::new();
        let b = CharacterId::new();
        assert!(a.as_uuid() <= b.as_uuid());
    }
}
