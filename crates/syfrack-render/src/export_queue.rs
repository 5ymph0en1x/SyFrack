//! Immutable tile storage and one atomic pair of cursors. Claiming either end
//! is linearizable, including the last tile: two independent counters are not.
use crate::Tile;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) struct TileWorkQueue {
    tiles: Box<[Tile]>,
    /// Low word = front, high word = exclusive back. Length is bounded by the
    /// u32 full-image pixel limit, checked before constructing an export.
    cursors: AtomicU64,
    /// Nanoseconds per pixel-sample, measured independently on each device.
    /// Zero means no observation yet. These are scheduling hints only.
    rates: [AtomicU64; 2],
    gpu_block_size: u32,
}

impl TileWorkQueue {
    #[cfg(test)]
    pub fn new(tiles: Vec<Tile>) -> Self {
        Self::with_gpu_block_size(tiles, 64)
    }

    pub fn with_gpu_block_size(mut tiles: Vec<Tile>, gpu_block_size: u32) -> Self {
        assert!([64, 128, 256].contains(&gpu_block_size));
        assert!(tiles.len() <= u32::MAX as usize);
        // Keep small CPU jobs adjacent inside larger GPU blocks. The host
        // checkpoint grid remains canonical row-major; only queue order changes.
        tiles.sort_unstable_by_key(|t| (t.y / gpu_block_size, t.x / gpu_block_size, t.y, t.x));
        let end = (tiles.len() as u64) << 32;
        Self {
            tiles: tiles.into_boxed_slice(),
            cursors: AtomicU64::new(end),
            rates: [AtomicU64::new(0), AtomicU64::new(0)],
            gpu_block_size,
        }
    }

    pub fn observe(&self, cpu: bool, pixels: u64, samples: u32, elapsed: Duration) {
        let work = pixels.saturating_mul(u64::from(samples)).max(1);
        let sample = (elapsed.as_nanos() / u128::from(work)).clamp(1, u64::MAX as u128) as u64;
        let rate = &self.rates[usize::from(!cpu)];
        // Smooth noisy dispatches, but respond immediately to slower tiles.
        let _ = rate.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
            Some(if old == 0 || sample >= old {
                sample
            } else {
                old - (old - sample) / 4
            })
        });
    }

    /// GPU consumes the front; CPU steals from the back. One claim grants the
    /// whole tile through its final sample; accumulations are never reduced.
    pub fn claim(&self, cpu: bool) -> Option<Tile> {
        self.claim_inner(cpu, false)
    }

    pub fn claim_gpu(&self) -> Option<Tile> {
        self.claim_inner(false, true)
    }

    fn claim_inner(&self, cpu: bool, batch: bool) -> Option<Tile> {
        let mut old = self.cursors.load(Ordering::Relaxed);
        loop {
            let front = old as u32;
            let back = (old >> 32) as u32;
            if front >= back {
                return None;
            }
            if cpu {
                let cpu_rate = self.rates[0].load(Ordering::Relaxed);
                let gpu_rate = self.rates[1].load(Ordering::Relaxed);
                // Leave the tail to the faster device when another CPU tile
                // would probably outlive the GPU queue. No claim is consumed:
                // the GPU still owns every unclaimed tile. Allow an initial
                // CPU tile to learn its rate; CPU-only queues have no GPU rate.
                if cpu_rate > 0
                    && gpu_rate > 0
                    && u64::from(back - front) <= cpu_rate.div_ceil(gpu_rate).saturating_mul(2)
                {
                    return None;
                }
            }
            let (tile, new) = if cpu {
                (
                    self.tiles[(back - 1) as usize],
                    (u64::from(back - 1) << 32) | u64::from(front),
                )
            } else {
                let mut tile = self.tiles[front as usize];
                let mut end = front + 1;
                if batch {
                    let first = tile;
                    let (mut x, mut y, mut right, mut bottom, mut area) =
                        (tile.x, tile.y, tile.x + tile.w, tile.y + tile.h, 0u64);
                    // At the minimum 8x8 cell size a 128x128 block has 256 cells.
                    let max_cells = (self.gpu_block_size / crate::tiles::MIN_TILE_SIZE).pow(2);
                    for i in front..back.min(front.saturating_add(max_cells)) {
                        let t = self.tiles[i as usize];
                        if (t.x / self.gpu_block_size, t.y / self.gpu_block_size)
                            != (first.x / self.gpu_block_size, first.y / self.gpu_block_size)
                        {
                            break;
                        }
                        x = x.min(t.x);
                        y = y.min(t.y);
                        right = right.max(t.x + t.w);
                        bottom = bottom.max(t.y + t.h);
                        area += u64::from(t.w) * u64::from(t.h);
                        // Complete rectangular prefixes only. A checkpoint hole
                        // or a concurrent CPU tail claim must never be shaded
                        // and committed a second time as part of a GPU block.
                        if right - x <= self.gpu_block_size
                            && bottom - y <= self.gpu_block_size
                            && area == u64::from(right - x) * u64::from(bottom - y)
                        {
                            tile = Tile {
                                x,
                                y,
                                w: right - x,
                                h: bottom - y,
                                done: 0,
                            };
                            end = i + 1;
                        }
                    }
                }
                (tile, (u64::from(back) << 32) | u64::from(end))
            };
            match self
                .cursors
                .compare_exchange_weak(old, new, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return Some(tile),
                Err(current) => old = current,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc, Barrier};
    #[test]
    fn opposite_consumers_never_duplicate_or_lose_the_last_tile() {
        for count in [0, 1, 2, 3, 257, 10_003] {
            let queue = Arc::new(TileWorkQueue::new(
                (0..count)
                    .map(|x| Tile {
                        x,
                        y: 0,
                        w: 1,
                        h: 1,
                        done: 0,
                    })
                    .collect(),
            ));
            let visits: Arc<Vec<_>> = Arc::new((0..count).map(|_| AtomicUsize::new(0)).collect());
            let barrier = Arc::new(Barrier::new(8));
            std::thread::scope(|scope| {
                for worker in 0..8 {
                    let (queue, visits, barrier) = (queue.clone(), visits.clone(), barrier.clone());
                    scope.spawn(move || {
                        barrier.wait();
                        while let Some(tile) = queue.claim(worker % 2 == 0) {
                            assert_eq!(visits[tile.x as usize].fetch_add(1, Ordering::Relaxed), 0);
                        }
                    });
                }
            });
            assert!(visits.iter().all(|v| v.load(Ordering::Relaxed) == 1));
            assert!(queue.claim(false).is_none() && queue.claim(true).is_none());
        }
    }

    #[test]
    fn slower_cpu_retires_without_consuming_or_losing_the_tail() {
        let queue = TileWorkQueue::new(
            (0..30)
                .map(|x| Tile {
                    x,
                    y: 0,
                    w: 1,
                    h: 1,
                    done: 0,
                })
                .collect(),
        );
        queue.observe(true, 1, 1, Duration::from_nanos(100));
        queue.observe(false, 1, 1, Duration::from_nanos(10));
        for expected in (20..30).rev() {
            assert_eq!(queue.claim(true).unwrap().x, expected);
        }
        assert!(queue.claim(true).is_none());
        for expected in 0..20 {
            assert_eq!(queue.claim(false).unwrap().x, expected);
        }
        assert!(queue.claim(false).is_none());
    }

    #[test]
    fn batched_gpu_and_cpu_cover_clipped_blocks_with_checkpoint_holes_once() {
        for (width, height) in [(37, 23), (129, 77), (64, 64)] {
            let mut tiles = crate::TileGrid::new(width, height, 8).tiles;
            tiles.retain(|t| !(t.x == 8 && t.y == 8));
            let expected = tiles.clone();
            let queue = TileWorkQueue::new(tiles);
            let barrier = Barrier::new(8);
            let visits: Vec<_> = (0..width * height).map(|_| AtomicUsize::new(0)).collect();
            std::thread::scope(|scope| {
                for worker in 0..8 {
                    let (queue, visits, barrier) = (&queue, &visits, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        while let Some(t) = if worker % 2 == 0 {
                            queue.claim(true)
                        } else {
                            queue.claim_gpu()
                        } {
                            assert!(t.w <= 64 && t.h <= 64);
                            for y in t.y..t.y + t.h {
                                for x in t.x..t.x + t.w {
                                    assert_eq!(
                                        visits[(y * width + x) as usize]
                                            .fetch_add(1, Ordering::Relaxed),
                                        0
                                    );
                                }
                            }
                        }
                    });
                }
            });
            for y in 0..height {
                for x in 0..width {
                    let present = expected
                        .iter()
                        .any(|t| x >= t.x && x < t.x + t.w && y >= t.y && y < t.y + t.h);
                    assert_eq!(
                        visits[(y * width + x) as usize].load(Ordering::Relaxed),
                        usize::from(present)
                    );
                }
            }
        }
    }

    #[test]
    fn gpu_groups_a_full_block_while_cpu_claims_one_small_tile() {
        let queue = TileWorkQueue::new(crate::TileGrid::new(129, 77, 16).tiles);
        let cpu = queue.claim(true).unwrap();
        assert_eq!((cpu.x, cpu.y, cpu.w, cpu.h), (128, 64, 1, 13));
        let gpu = queue.claim_gpu().unwrap();
        assert_eq!((gpu.x, gpu.y, gpu.w, gpu.h), (0, 0, 64, 64));
    }

    #[test]
    fn larger_gpu_blocks_cover_clipped_cells_and_checkpoint_holes_exactly_once() {
        for (base, block) in [(8, 128), (16, 128), (8, 256), (16, 256)] {
            let tiles: Vec<_> = crate::TileGrid::new(271, 145, base)
                .tiles
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i % 17 != 3)
                .map(|(_, t)| t)
                .collect();
            let expected: u64 = tiles.iter().map(|t| u64::from(t.w) * u64::from(t.h)).sum();
            let queue = TileWorkQueue::with_gpu_block_size(tiles, block);
            let mut visits = vec![0u8; 271 * 145];
            let mut total = 0;
            loop {
                let mut claimed = false;
                for tile in [queue.claim(true), queue.claim_gpu()].into_iter().flatten() {
                    claimed = true;
                    assert!(tile.w <= block && tile.h <= block);
                    for y in tile.y..tile.y + tile.h {
                        for x in tile.x..tile.x + tile.w {
                            let n = &mut visits[(y * 271 + x) as usize];
                            *n += 1;
                            assert_eq!(*n, 1);
                            total += 1;
                        }
                    }
                }
                if !claimed {
                    break;
                }
            }
            assert_eq!(total, expected);
        }
        for block in [128, 256] {
            let queue =
                TileWorkQueue::with_gpu_block_size(crate::TileGrid::new(512, 512, 8).tiles, block);
            let tile = queue.claim_gpu().unwrap();
            assert_eq!((tile.w, tile.h), (block, block));
        }
    }
}
