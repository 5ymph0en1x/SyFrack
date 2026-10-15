pub const MIN_TILE_SIZE: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Samples per pixel accumulated so far in this tile.
    pub done: u32,
}

#[derive(Clone, Debug)]
pub struct TileGrid {
    pub width: u32,
    pub height: u32,
    /// Base cell size. Adaptive tiles may cover several cells.
    pub tile_size: u32,
    pub tiles: Vec<Tile>,
}

impl TileGrid {
    pub fn new(width: u32, height: u32, tile_size: u32) -> Self {
        let tile_size = tile_size.max(MIN_TILE_SIZE);
        let mut tiles = Vec::new();
        let mut y = 0;
        while y < height {
            let h = tile_size.min(height - y);
            let mut x = 0;
            while x < width {
                let w = tile_size.min(width - x);
                tiles.push(Tile {
                    x,
                    y,
                    w,
                    h,
                    done: 0,
                });
                x += w;
            }
            y += h;
        }
        Self {
            width,
            height,
            tile_size,
            tiles,
        }
    }

    /// Index of the least-advanced tile still below `target`, lowest index on ties.
    pub fn next_pending(&self, target: u32) -> Option<usize> {
        self.tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.done < target)
            .min_by_key(|(i, t)| (t.done, *i))
            .map(|(i, _)| i)
    }

    pub fn min_done(&self) -> u32 {
        self.tiles.iter().map(|t| t.done).min().unwrap_or(0)
    }

    pub fn max_done(&self) -> u32 {
        self.tiles.iter().map(|t| t.done).max().unwrap_or(0)
    }

    pub fn at_pass_boundary(&self) -> bool {
        self.min_done() == self.max_done()
    }

    /// Restore a uniform or adaptive layout, checking complete, disjoint coverage.
    /// Validation uses base cells instead of a potentially enormous pixel bitmap.
    pub fn restore(
        width: u32,
        height: u32,
        tile_size: u32,
        tiles: Vec<Tile>,
        target: u32,
    ) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || tile_size < MIN_TILE_SIZE {
            return Err("invalid grid dimensions");
        }
        let cols = width.div_ceil(tile_size);
        let rows = height.div_ceil(tile_size);
        let mut covered = vec![false; (u64::from(cols) * u64::from(rows)) as usize];
        for t in &tiles {
            let right = t.x.checked_add(t.w).ok_or("tile extent overflow")?;
            let bottom = t.y.checked_add(t.h).ok_or("tile extent overflow")?;
            if t.w == 0
                || t.h == 0
                || right > width
                || bottom > height
                || t.done > target
                || t.x % tile_size != 0
                || t.y % tile_size != 0
                || (right != width && right % tile_size != 0)
                || (bottom != height && bottom % tile_size != 0)
            {
                return Err("invalid tile bounds or progress");
            }
            for y in t.y / tile_size..bottom.div_ceil(tile_size) {
                for x in t.x / tile_size..right.div_ceil(tile_size) {
                    let cell =
                        &mut covered[(u64::from(y) * u64::from(cols) + u64::from(x)) as usize];
                    if *cell {
                        return Err("overlapping tiles");
                    }
                    *cell = true;
                }
            }
        }
        if covered.iter().any(|c| !c) {
            return Err("incomplete tile coverage");
        }
        Ok(Self {
            width,
            height,
            tile_size,
            tiles,
        })
    }

    /// Merge only inexpensive sibling tiles, using their measured total cost.
    /// Expensive regions keep small dispatches without holding back the rest.
    /// One level per observation round prevents extrapolating unmeasured work.
    pub fn merge_fast(&mut self, seconds: &[f64], max_size: u32, budget: f64) -> bool {
        if !self.at_pass_boundary()
            || seconds.len() != self.tiles.len()
            || !budget.is_finite()
            || budget <= 0.0
        {
            return false;
        }
        let lookup: std::collections::HashMap<_, _> = self
            .tiles
            .iter()
            .enumerate()
            .map(|(i, t)| ((t.x, t.y, t.w, t.h), i))
            .collect();
        let mut used = vec![false; self.tiles.len()];
        let mut merged = Vec::new();
        let mut size = self.tile_size;
        while let Some(parent) = size.checked_mul(2).filter(|s| *s <= max_size) {
            for y in (0..self.height).step_by(parent as usize) {
                for x in (0..self.width).step_by(parent as usize) {
                    let mut siblings = Vec::with_capacity(4);
                    let mut cost = 0.0;
                    let mut valid = true;
                    for dy in [0, size] {
                        for dx in [0, size] {
                            let (sx, sy) = (x + dx, y + dy);
                            if sx >= self.width || sy >= self.height {
                                continue;
                            }
                            let key = (
                                sx,
                                sy,
                                size.min(self.width - sx),
                                size.min(self.height - sy),
                            );
                            match lookup.get(&key).copied() {
                                Some(i)
                                    if !used[i] && seconds[i].is_finite() && seconds[i] > 0.0 =>
                                {
                                    siblings.push(i);
                                    cost += seconds[i];
                                }
                                _ => valid = false,
                            }
                        }
                    }
                    if valid && siblings.len() > 1 && cost < budget {
                        for i in siblings {
                            used[i] = true;
                        }
                        merged.push(Tile {
                            x,
                            y,
                            w: parent.min(self.width - x),
                            h: parent.min(self.height - y),
                            done: self.min_done(),
                        });
                    }
                }
            }
            size = parent;
        }
        if merged.is_empty() {
            return false;
        }
        merged.extend(
            self.tiles
                .iter()
                .enumerate()
                .filter(|(i, _)| !used[*i])
                .map(|(_, t)| *t),
        );
        merged.sort_unstable_by_key(|t| (t.y, t.x));
        self.tiles = merged;
        true
    }

    /// Row-major progress on the base grid, also for adaptive rectangles.
    pub fn progress_cells(&self) -> Vec<u32> {
        let cols = self.width.div_ceil(self.tile_size);
        let rows = self.height.div_ceil(self.tile_size);
        let mut cells = vec![0; (cols * rows) as usize];
        for t in &self.tiles {
            for y in t.y / self.tile_size..(t.y + t.h).div_ceil(self.tile_size) {
                for x in t.x / self.tile_size..(t.x + t.w).div_ceil(self.tile_size) {
                    cells[(y * cols + x) as usize] = t.done;
                }
            }
        }
        cells
    }

    /// Halves the tile size, keeping the (uniform) progress. Only legal at a
    /// pass boundary, because progress is tracked per tile.
    pub fn halve(&mut self) -> bool {
        if !self.at_pass_boundary() {
            return false;
        }
        let done = self.min_done();
        let mut next = Self::new(
            self.width,
            self.height,
            (self.tile_size / 2).max(MIN_TILE_SIZE),
        );
        for t in &mut next.tiles {
            t.done = done;
        }
        if self.tiles == next.tiles {
            return false;
        }
        *self = next;
        true
    }

    /// Doubles the tile size, keeping the (uniform) progress. Mirror of
    /// `halve`; only legal at a pass boundary.
    pub fn double(&mut self) -> bool {
        if !self.at_pass_boundary() {
            return false;
        }
        let done = self.min_done();
        let mut next = Self::new(self.width, self.height, self.tile_size * 2);
        for t in &mut next.tiles {
            t.done = done;
        }
        *self = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_coverage(g: &TileGrid) {
        let mut covered = vec![0u8; (g.width * g.height) as usize];
        for t in &g.tiles {
            for y in t.y..t.y + t.h {
                for x in t.x..t.x + t.w {
                    covered[(y * g.width + x) as usize] += 1;
                }
            }
        }
        assert!(covered.iter().all(|c| *c == 1));
        TileGrid::restore(g.width, g.height, g.tile_size, g.tiles.clone(), 10).unwrap();
    }

    #[test]
    fn adaptive_growth_keeps_expensive_regions_small() {
        let mut g = TileGrid::new(64, 64, 16);
        for t in &mut g.tiles {
            t.done = 3;
        }
        let mut costs = vec![0.01; 16];
        costs[0] = 0.18;
        assert!(g.merge_fast(&costs, 64, 0.2));
        assert_eq!(g.tiles.len(), 7);
        assert_eq!(g.tiles[0].w, 16);
        assert!(g.tiles.iter().all(|t| t.done == 3));
        assert_eq!(g.progress_cells(), vec![3; 16]);
        check_coverage(&g);
        assert!(!g.merge_fast(&vec![0.1; 7], 64, 0.2));
        assert!(g.halve());
        check_coverage(&g);
    }

    #[test]
    fn adaptive_growth_preserves_clipped_edges_and_rounds() {
        for (w, h, base) in [(70, 51, 16), (100, 67, 10), (7, 29, 8)] {
            let mut g = TileGrid::new(w, h, base);
            for _ in 0..3 {
                g.merge_fast(&vec![0.01; g.tiles.len()], 64, 0.2);
                check_coverage(&g);
            }
        }
        let mut g = TileGrid::new(64, 64, 16);
        g.tiles[0].done = 1;
        assert!(!g.merge_fast(&vec![0.01; 16], 64, 0.2));
        g.tiles[0].done = 0;
        for costs in [vec![0.0; 16], vec![f64::NAN; 16], vec![0.1; 16]] {
            assert!(!g.merge_fast(&costs, 64, 0.2));
        }
        assert!(!g.merge_fast(&vec![0.01; 16], 16, 0.2));
    }

    #[test]
    fn restored_layout_rejects_overlap_holes_bad_bounds_and_progress() {
        let g = TileGrid::new(64, 64, 16);
        let mut bad = g.tiles.clone();
        bad[1] = bad[0];
        assert!(TileGrid::restore(64, 64, 16, bad, 4).is_err());
        assert!(TileGrid::restore(64, 64, 16, g.tiles[..15].to_vec(), 4).is_err());
        for t in [
            Tile { x: 1, ..g.tiles[0] },
            Tile {
                w: 65,
                ..g.tiles[0]
            },
            Tile { w: 0, ..g.tiles[0] },
            Tile {
                done: 5,
                ..g.tiles[0]
            },
            Tile {
                x: u32::MAX,
                ..g.tiles[0]
            },
        ] {
            let mut bad = g.tiles.clone();
            bad[0] = t;
            assert!(TileGrid::restore(64, 64, 16, bad, 4).is_err());
        }
    }

    #[test]
    fn grid_covers_the_image_exactly_once() {
        let g = TileGrid::new(100, 70, 32);
        assert_eq!(g.tiles.len(), 4 * 3);
        let mut covered = vec![0u8; 100 * 70];
        for t in &g.tiles {
            for y in t.y..t.y + t.h {
                for x in t.x..t.x + t.w {
                    covered[(y * 100 + x) as usize] += 1;
                }
            }
        }
        assert!(covered.iter().all(|c| *c == 1));
    }

    #[test]
    fn next_pending_picks_the_least_advanced_tile() {
        let mut g = TileGrid::new(64, 32, 32);
        assert_eq!(g.next_pending(4), Some(0));
        g.tiles[0].done = 2;
        assert_eq!(g.next_pending(4), Some(1));
        g.tiles[1].done = 2;
        assert_eq!(g.next_pending(4), Some(0));
        g.tiles[0].done = 4;
        g.tiles[1].done = 4;
        assert_eq!(g.next_pending(4), None);
    }

    #[test]
    fn pass_boundary_means_all_tiles_equal() {
        let mut g = TileGrid::new(64, 64, 32);
        assert!(g.at_pass_boundary());
        g.tiles[2].done = 1;
        assert!(!g.at_pass_boundary());
        for t in &mut g.tiles {
            t.done = 1;
        }
        assert!(g.at_pass_boundary());
        assert_eq!(g.min_done(), 1);
        assert_eq!(g.max_done(), 1);
    }

    #[test]
    fn halve_rebuilds_with_smaller_tiles_and_keeps_progress() {
        let mut g = TileGrid::new(128, 128, 64);
        for t in &mut g.tiles {
            t.done = 3;
        }
        assert!(g.halve());
        assert_eq!(g.tile_size, 32);
        assert_eq!(g.tiles.len(), 16);
        assert!(g.tiles.iter().all(|t| t.done == 3));
    }

    #[test]
    fn double_mirrors_halve_and_keeps_progress() {
        let mut g = TileGrid::new(128, 128, 32);
        g.tiles[0].done = 1;
        assert!(!g.double());
        for t in &mut g.tiles {
            t.done = 2;
        }
        assert!(g.double());
        assert_eq!(g.tile_size, 64);
        assert_eq!(g.tiles.len(), 4);
        assert!(g.tiles.iter().all(|t| t.done == 2));
    }

    #[test]
    fn halve_refuses_outside_pass_boundary_or_below_minimum() {
        let mut g = TileGrid::new(64, 64, 32);
        g.tiles[0].done = 1;
        assert!(!g.halve());
        let mut small = TileGrid::new(64, 64, MIN_TILE_SIZE);
        assert!(!small.halve(), "minimum tile size");
    }
}
