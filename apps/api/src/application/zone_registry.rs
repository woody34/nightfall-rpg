//! Routes players to the running zone (plan §8 #13).
//!
//! Production wraps the durable `ZoneBootstrap` handle; tests can start an in-memory fixture.

use crate::domain::zone::{
    Fixed, InvalidBounds, Vec2Fixed, ZoneBounds, ZoneId, ZoneSeed, ZoneState,
};
use crate::domain::CharacterId;

use super::zone_actor::{TickSource, ZoneActor, ZoneHandle};

/// The fixture zone's id.
pub const FIXTURE_ZONE: ZoneId = ZoneId(1);

/// The fixture zone is this many tiles on each side, `(0, 0)` to `(256, 256)` inclusive: eight
/// 32-tile AOI cells across, so two players can stand outside each other's 3x3 AOI.
pub const FIXTURE_ZONE_TILES: i32 = 256;

/// The running zones.
#[derive(Debug, Clone)]
pub struct ZoneRegistry {
    fixture: ZoneHandle,
}

impl ZoneRegistry {
    /// Starts the fixture zone on `ticks`. `time_origin_ms` is the wall-clock time of tick 0
    /// (Unix ms); it also numbers the epoch, so every server start is a new epoch (plan §8
    /// #5). Must be called inside a tokio runtime.
    pub fn start_fixture<T: TickSource>(
        time_origin_ms: i64,
        ticks: T,
    ) -> Result<Self, InvalidBounds> {
        Ok(Self {
            fixture: ZoneActor::spawn(fixture_state(time_origin_ms)?, ticks),
        })
    }

    /// Wraps an already running zone, including the durable bootstrap's handle.
    #[must_use]
    pub const fn from_handle(fixture: ZoneHandle) -> Self {
        Self { fixture }
    }

    /// The zone `character` plays in. Every character is in the fixture zone for now.
    #[must_use]
    pub const fn zone_for(&self, _character: CharacterId) -> &ZoneHandle {
        &self.fixture
    }

    /// The fixture zone.
    #[must_use]
    pub const fn fixture(&self) -> &ZoneHandle {
        &self.fixture
    }
}

/// An empty fixture zone at tick 0.
pub fn fixture_state(time_origin_ms: i64) -> Result<ZoneState, InvalidBounds> {
    let seed = ZoneSeed {
        zone: FIXTURE_ZONE,
        epoch: u64::try_from(time_origin_ms).unwrap_or_default(),
    };
    let max = Fixed::from_tiles(FIXTURE_ZONE_TILES);
    let bounds = ZoneBounds::new(Vec2Fixed::default(), Vec2Fixed::new(max, max))?;
    Ok(ZoneState::new(seed, bounds, time_origin_ms))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn fixture_zone_is_256_tiles_square_and_numbers_its_epoch_from_the_origin() {
        let z = fixture_state(1_700_000_000_000).unwrap();
        assert_eq!(z.seed().zone, FIXTURE_ZONE);
        assert_eq!(z.seed().epoch, 1_700_000_000_000);
        assert!(z.bounds().contains(Vec2Fixed::from_tiles(256, 256)));
        assert!(z.bounds().contains(Vec2Fixed::from_tiles(0, 0)));
        assert!(!z.bounds().contains(Vec2Fixed::from_tiles(257, 0)));
        assert!(!z.bounds().contains(Vec2Fixed::from_tiles(0, -1)));
    }
}
