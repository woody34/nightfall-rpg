//! Deterministic zone simulation (plan D7, docs/plans/phase-0b-connected-slice.md Story 3.1).
//!
//! The same snapshot plus the same commands on the same ticks must produce byte-identical
//! events on any machine and any build, so replay (Story 3.3) can prove a recorded session.
//! Rules for everything under this module (docs/engineering/rust-guidelines.md, Determinism):
//!
//! * No floats. Positions are [`Fixed`] (1/1000 tile, `i32`); distances compare squared
//!   integers. Conversion to wire floats lives in `interface::zone_mapping`.
//! * No hash-ordered collections. `BTreeMap` / `BTreeSet` only, so iteration order is defined.
//! * No wall clock. Time is the [`Tick`] passed in by the caller.
//! * No ambient randomness. The only generator is the `ChaCha12` stream seeded from
//!   [`ZoneSeed`] that [`ZoneState`] owns.
//!
//! The lints below turn the float rule into a compile error under clippy, and
//! `tests::zone_sources_use_no_nondeterministic_apis` scans these files for the rest.

#![deny(
    clippy::float_arithmetic,
    clippy::float_cmp,
    clippy::float_cmp_const,
    clippy::lossy_float_literal,
    clippy::cast_precision_loss,
    clippy::imprecise_flops,
    clippy::suboptimal_flops
)]

mod aoi;
mod command;
mod entity;
mod fixed;
mod state;

pub use aoi::{aoi_cells_for, AoiCell, AoiIndex, CellCoord, AOI_CELL_TILES};
pub use command::{
    AppliedCommand, AppliedTick, AppliedTickDraft, CommandSource, Disposition, ObserverOutput,
    Ordinal, RejectReason, SessionGeneration, ZoneCommand, ZoneEvent, ZoneInput,
};
pub use entity::{Entity, EntityId, EntityKind, Tick, TICK_MS};
pub use fixed::{Fixed, Speed, Vec2Fixed, UNITS_PER_TILE};
pub use state::{
    InvalidBounds, RngState, SnapshotError, SnapshotMeta, TickError, ZoneBounds, ZoneId, ZoneSeed,
    ZoneSnapshot, ZoneState, MAX_MOVE_DISTANCE_TILES, SNAPSHOT_SCHEMA_VERSION,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    /// Every source file in this module, embedded at compile time so the scan cannot miss a
    /// file that exists but is not listed: adding a module without adding it here fails
    /// `every_zone_source_is_scanned`.
    const SOURCES: [(&str, &str); 7] = [
        ("mod.rs", include_str!("mod.rs")),
        ("aoi.rs", include_str!("aoi.rs")),
        ("command.rs", include_str!("command.rs")),
        ("entity.rs", include_str!("entity.rs")),
        ("fixed.rs", include_str!("fixed.rs")),
        ("state.rs", include_str!("state.rs")),
        ("state_tests.rs", include_str!("state_tests.rs")),
    ];

    /// Tokens whose presence would make the zone nondeterministic. Built by concatenation so
    /// this list does not trip its own scan.
    fn forbidden() -> Vec<String> {
        [
            ("f3", "2"),
            ("f6", "4"),
            ("Hash", "Map"),
            ("Hash", "Set"),
            ("Inst", "ant"),
            ("System", "Time"),
            ("thread_", "rng"),
            ("Os", "Rng"),
            ("rand::", "random"),
            ("from_", "entropy"),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}"))
        .collect()
    }

    #[test]
    fn zone_sources_use_no_nondeterministic_apis() {
        let forbidden = forbidden();
        for (file, src) in SOURCES {
            for (n, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for token in &forbidden {
                    assert!(
                        !code.contains(token.as_str()),
                        "domain/zone/{file}:{} uses `{token}`, which breaks determinism (plan D7)",
                        n + 1
                    );
                }
            }
        }
    }

    #[test]
    fn every_zone_source_is_scanned() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/zone");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| {
                std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e == "rs")
            })
            .collect();
        on_disk.sort();
        let mut scanned: Vec<String> = SOURCES.iter().map(|(n, _)| (*n).to_owned()).collect();
        scanned.sort();
        assert_eq!(on_disk, scanned);
    }
}
