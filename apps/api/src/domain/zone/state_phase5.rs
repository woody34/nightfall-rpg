//! V7 draw primitives on the zone RNG stream (design §2.3): `draw_u64` (high word first),
//! `chance` and `chance_bound` over `U256`, beside the existing `roll_below`. Every call
//! consumes exactly its words whatever the outcome (rule P3) and is recorded as a raw draw.
//! Stories 3.3–3.11 install the callsites; nothing here runs in a V5/V6 epoch.

use crate::domain::zone::binary64::Binary64;
use crate::domain::zone::wide::{chance_binary64, chance_bound, WideError};
use crate::domain::zone::{Phase5Snapshot, ZoneState};
use rand_chacha::rand_core::Rng as _;

impl ZoneState {
    /// `draw_u64()`: `hi = next_u32(); lo = next_u32(); (hi << 32) | lo` (design §2.3).
    /// Used only for E15 chance tests.
    pub fn draw_u64(&mut self) -> u64 {
        let before = self.rng.get_word_pos();
        let hi = self.rng.next_u32();
        let lo = self.rng.next_u32();
        self.note_raw_draw("econ_draw_u64_hi", before, 0, hi);
        self.note_raw_draw("econ_draw_u64_lo", before, 0, lo);
        (u64::from(hi) << 32) | u64::from(lo)
    }

    /// `chance(C)` of design §2.3 for a binary64 percent chance: one `draw_u64`, success iff
    /// `r · (100·den) < num · 2^64` in `U256`. The draw is taken even at 0 or ≥ 100 (P3).
    pub fn chance(&mut self, chance: Binary64) -> Result<bool, WideError> {
        let draw = self.draw_u64();
        chance_binary64(chance, draw)
    }

    /// `chance_bound(k, n)` of design §2.3: the integer `Rnd.get(n) < k` under E15.
    pub fn chance_bound(&mut self, k: u64, n: u64) -> Result<bool, WideError> {
        let draw = self.draw_u64();
        chance_bound(k, n, draw)
    }

    /// The V7 block, if this is a V7 epoch.
    #[must_use]
    pub fn phase5_block(&self) -> Option<&Phase5Snapshot> {
        self.phase5.as_ref().map(|p| &p.canonical)
    }

    /// Test support: replace the canonical V7 block without validation, to digest and
    /// encode arbitrary contents. Production code never bypasses the invariants.
    pub fn set_phase5_block_for_test(&mut self, block: Phase5Snapshot) {
        if let Some(phase5) = &mut self.phase5 {
            phase5.canonical = block;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::domain::zone::binary64::fl_decimal;
    use crate::domain::zone::Vec2Fixed;
    use crate::domain::zone::{ZoneBounds, ZoneId, ZoneSeed};

    fn zone() -> ZoneState {
        ZoneState::new(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(64, 64)).unwrap(),
            0,
        )
    }

    #[test]
    fn draw_u64_is_two_words_high_first_and_always_consumed() {
        let mut a = zone();
        let mut b = zone();
        let hi = b.rng.next_u32();
        let lo = b.rng.next_u32();
        let before = a.rng.get_word_pos();
        assert_eq!(a.draw_u64(), (u64::from(hi) << 32) | u64::from(lo));
        assert_eq!(a.rng.get_word_pos(), before + 2);
        // Chance at 0 and at 100 both consume exactly one draw_u64 (P3).
        let before = a.rng.get_word_pos();
        assert!(!a.chance(Binary64::ZERO).unwrap());
        assert!(a.chance(fl_decimal("100").unwrap()).unwrap());
        assert_eq!(a.rng.get_word_pos(), before + 4);
        assert!(a.chance_bound(100, 100).unwrap());
        assert!(!a.chance_bound(0, 100).unwrap());
        assert!(a.chance_bound(1, 0).is_err());
        assert_eq!(a.rng.get_word_pos(), before + 10);
    }

    /// Design §14.4 / story 3.2: no `u128`/`i128` chance product in the Phase 5 engine.
    #[test]
    fn phase5_engine_sources_use_no_128_bit_chance_arithmetic() {
        for (name, source) in [
            ("wide.rs", include_str!("wide.rs")),
            ("binary64.rs", include_str!("binary64.rs")),
            ("phase5.rs", include_str!("phase5.rs")),
            ("phase5_rules.rs", include_str!("phase5_rules.rs")),
            ("state_phase5.rs", include_str!("state_phase5.rs")),
            ("state_digest_phase5.rs", include_str!("state_digest_phase5.rs")),
        ] {
            let production = source.split("#[cfg(test)]").next().unwrap_or("");
            // The forbidden tokens are assembled so this scan passes the zone-wide
            // nondeterminism scan itself.
            let wide = [["u12", "8"].concat(), ["i12", "8"].concat()];
            for forbidden in &wide {
                assert!(
                    !production.contains(forbidden.as_str()),
                    "{name}: {forbidden} in the Phase 5 engine (use U256 and BigUint)"
                );
            }
            let floats = [["f6", "4"].concat(), ["f3", "2"].concat()];
            for forbidden in &floats {
                assert!(!production.contains(forbidden.as_str()), "{name}: float arithmetic");
            }
        }
    }
}
