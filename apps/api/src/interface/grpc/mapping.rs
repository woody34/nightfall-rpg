//! Proto <-> domain conversions. Pure functions, unit-tested.

use super::pb;
use crate::domain::{Character, Race};

pub(super) fn race_to_pb(r: Race) -> pb::Race {
    match r {
        Race::Human => pb::Race::Human,
        Race::Elf => pb::Race::Elf,
        Race::DarkElf => pb::Race::DarkElf,
        Race::Orc => pb::Race::Orc,
        Race::Dwarf => pb::Race::Dwarf,
    }
}

/// `None` for `RACE_UNSPECIFIED` or an unknown enum number.
pub(super) fn race_from_pb(n: i32) -> Option<Race> {
    match pb::Race::try_from(n).ok()? {
        pb::Race::Human => Some(Race::Human),
        pb::Race::Elf => Some(Race::Elf),
        pb::Race::DarkElf => Some(Race::DarkElf),
        pb::Race::Orc => Some(Race::Orc),
        pb::Race::Dwarf => Some(Race::Dwarf),
        pb::Race::Unspecified => None,
    }
}

pub(super) fn character_to_pb(c: &Character) -> pb::Character {
    pb::Character {
        id: c.id.to_string(),
        name: c.name.as_str().to_owned(),
        race: race_to_pb(c.race) as i32,
        level: c.level,
        stats: Some(pb::BaseStats {
            str: c.stats.str,
            dex: c.stats.dex,
            con: c.stats.con,
            int: c.stats.int,
            wit: c.stats.wit,
            men: c.stats.men,
        }),
        position: Some(pb::Position {
            x: c.position.x,
            y: c.position.y,
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{AccountId, CharacterName};

    #[test]
    fn race_round_trips_and_rejects_unspecified() {
        for r in [
            Race::Human,
            Race::Elf,
            Race::DarkElf,
            Race::Orc,
            Race::Dwarf,
        ] {
            assert_eq!(race_from_pb(race_to_pb(r) as i32), Some(r));
        }
        assert_eq!(race_from_pb(0), None);
        assert_eq!(race_from_pb(999), None);
    }

    #[test]
    fn character_maps_every_field() {
        let c = Character::create(
            AccountId::from_uuid(Uuid::nil()),
            CharacterName::new("Frodo").unwrap(),
            Race::Dwarf,
        );
        let p = character_to_pb(&c);
        assert_eq!(p.id, c.id.to_string());
        assert_eq!(p.name, "Frodo");
        assert_eq!(p.race, pb::Race::Dwarf as i32);
        assert_eq!(p.level, 1);
        assert_eq!(p.stats.unwrap().con, Race::Dwarf.starting_stats().con);
        assert!(p.position.is_some());
    }
}
