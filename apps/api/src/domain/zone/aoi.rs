//! Area-of-interest grid (docs/planning/06-world-and-content.md §3.1): the zone plane is cut
//! into square cells of [`AOI_CELL_TILES`] tiles; an entity sees the 3x3 block of cells around
//! its own. Ordered maps only, so iterating the index is deterministic.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::entity::EntityId;
use super::fixed::{Fixed, Vec2Fixed};

/// Cell edge length in tiles.
pub const AOI_CELL_TILES: i32 = 32;

/// Cell edge length in fixed-point units.
const CELL_UNITS: i32 = Fixed::from_tiles(AOI_CELL_TILES).raw();

/// Integer coordinates of one AOI cell. Cell `(0, 0)` covers `[0, 32)` tiles on both axes;
/// negative positions floor toward negative infinity, so `-0.001` tiles is in cell `-1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellCoord {
    /// Column.
    pub x: i32,
    /// Row.
    pub y: i32,
}

impl CellCoord {
    /// The cell containing `pos`.
    #[must_use]
    pub const fn of(pos: Vec2Fixed) -> Self {
        Self {
            x: pos.x.raw().div_euclid(CELL_UNITS),
            y: pos.y.raw().div_euclid(CELL_UNITS),
        }
    }

    /// This cell and its eight neighbours in a fixed row-major order: `y - 1` row first, and
    /// within a row `x - 1`, `x`, `x + 1`. Index 4 is always `self`. Cell coordinates are at
    /// most `i32::MAX / 32000` in magnitude, so the +/-1 never saturates in practice.
    #[must_use]
    pub const fn neighbourhood(self) -> [Self; 9] {
        let (x0, x1, x2) = (self.x.saturating_sub(1), self.x, self.x.saturating_add(1));
        let (y0, y1, y2) = (self.y.saturating_sub(1), self.y, self.y.saturating_add(1));
        [
            Self { x: x0, y: y0 },
            Self { x: x1, y: y0 },
            Self { x: x2, y: y0 },
            Self { x: x0, y: y1 },
            Self { x: x1, y: y1 },
            Self { x: x2, y: y1 },
            Self { x: x0, y: y2 },
            Self { x: x1, y: y2 },
            Self { x: x2, y: y2 },
        ]
    }
}

/// The 3x3 AOI neighbourhood of `pos`, in [`CellCoord::neighbourhood`] order.
#[must_use]
pub const fn aoi_cells_for(pos: Vec2Fixed) -> [CellCoord; 9] {
    CellCoord::of(pos).neighbourhood()
}

/// One non-empty cell of the index, as stored in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AoiCell {
    /// The cell.
    pub cell: CellCoord,
    /// Its entities in id order.
    pub entities: Vec<EntityId>,
}

/// Which entities are in which cell. Kept in step with entity positions by `ZoneState`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AoiIndex {
    cells: BTreeMap<CellCoord, BTreeSet<EntityId>>,
}

impl AoiIndex {
    /// Records `id` in the cell containing `pos`.
    pub fn insert(&mut self, id: EntityId, pos: Vec2Fixed) {
        self.cells.entry(CellCoord::of(pos)).or_default().insert(id);
    }

    /// Removes `id` from the cell containing `pos`; drops the cell when it empties.
    pub fn remove(&mut self, id: EntityId, pos: Vec2Fixed) {
        let cell = CellCoord::of(pos);
        if let Some(set) = self.cells.get_mut(&cell) {
            set.remove(&id);
            if set.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }

    /// Re-buckets `id` if the move crossed a cell edge.
    pub fn relocate(&mut self, id: EntityId, from: Vec2Fixed, to: Vec2Fixed) {
        if CellCoord::of(from) != CellCoord::of(to) {
            self.remove(id, from);
            self.insert(id, to);
        }
    }

    /// Entities in one cell, in id order.
    pub fn in_cell(&self, cell: CellCoord) -> impl Iterator<Item = EntityId> + '_ {
        self.cells.get(&cell).into_iter().flatten().copied()
    }

    /// Entities in the 3x3 neighbourhood of `pos`: cells in neighbourhood order, ids in id
    /// order within a cell.
    pub fn in_aoi(&self, pos: Vec2Fixed) -> impl Iterator<Item = EntityId> + '_ {
        aoi_cells_for(pos).into_iter().flat_map(|c| self.in_cell(c))
    }

    /// The index as plain data, cells and ids in order.
    #[must_use]
    pub fn to_cells(&self) -> Vec<AoiCell> {
        self.cells
            .iter()
            .map(|(cell, ids)| AoiCell {
                cell: *cell,
                entities: ids.iter().copied().collect(),
            })
            .collect()
    }

    /// Number of non-empty cells.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn at(x: i32, y: i32) -> Vec2Fixed {
        Vec2Fixed::new(Fixed::from_raw(x), Fixed::from_raw(y))
    }

    #[test]
    fn cell_edges_belong_to_the_higher_cell() {
        assert_eq!(CellCoord::of(at(0, 0)), CellCoord { x: 0, y: 0 });
        assert_eq!(CellCoord::of(at(31_999, 31_999)), CellCoord { x: 0, y: 0 });
        assert_eq!(CellCoord::of(at(32_000, 0)), CellCoord { x: 1, y: 0 });
        assert_eq!(CellCoord::of(at(0, 32_000)), CellCoord { x: 0, y: 1 });
    }

    #[test]
    fn negative_positions_floor_toward_negative_infinity() {
        assert_eq!(CellCoord::of(at(-1, -1)), CellCoord { x: -1, y: -1 });
        assert_eq!(CellCoord::of(at(-32_000, 0)), CellCoord { x: -1, y: 0 });
        assert_eq!(CellCoord::of(at(-32_001, 0)), CellCoord { x: -2, y: 0 });
    }

    #[test]
    fn extreme_positions_have_valid_cells() {
        let lo = CellCoord::of(at(i32::MIN, i32::MIN));
        let hi = CellCoord::of(at(i32::MAX, i32::MAX));
        assert_eq!(
            lo,
            CellCoord {
                x: -67_109,
                y: -67_109
            }
        );
        assert_eq!(
            hi,
            CellCoord {
                x: 67_108,
                y: 67_108
            }
        );
        assert_eq!(
            lo.neighbourhood()[0],
            CellCoord {
                x: -67_110,
                y: -67_110
            }
        );
    }

    #[test]
    fn neighbourhood_is_row_major_with_self_in_the_middle() {
        let n = aoi_cells_for(at(32_000, 64_000));
        assert_eq!(n[4], CellCoord { x: 1, y: 2 });
        assert_eq!(n[0], CellCoord { x: 0, y: 1 });
        assert_eq!(n[2], CellCoord { x: 2, y: 1 });
        assert_eq!(n[8], CellCoord { x: 2, y: 3 });
        let mut sorted = n;
        sorted.sort_by_key(|c| (c.y, c.x));
        assert_eq!(sorted, n);
    }

    #[test]
    fn index_tracks_moves_across_cell_edges() {
        let a = EntityId(Uuid::from_u128(1));
        let b = EntityId(Uuid::from_u128(2));
        let mut idx = AoiIndex::default();
        idx.insert(a, at(31_999, 0));
        idx.insert(b, at(100_000, 0));
        assert_eq!(idx.in_cell(CellCoord { x: 0, y: 0 }).collect::<Vec<_>>(), vec![a]);

        idx.relocate(a, at(31_999, 0), at(32_000, 0));
        assert_eq!(idx.in_cell(CellCoord { x: 0, y: 0 }).count(), 0);
        assert_eq!(idx.in_cell(CellCoord { x: 1, y: 0 }).collect::<Vec<_>>(), vec![a]);
        assert_eq!(idx.cell_count(), 2);

        // b (cell 3) is two cells from a (cell 1): outside a's AOI, inside cell 2's.
        assert_eq!(idx.in_aoi(at(32_000, 0)).collect::<Vec<_>>(), vec![a]);
        assert_eq!(idx.in_aoi(at(64_000, 0)).collect::<Vec<_>>(), vec![a, b]);

        idx.remove(a, at(32_000, 0));
        idx.remove(b, at(100_000, 0));
        assert_eq!(idx.cell_count(), 0);
    }
}
