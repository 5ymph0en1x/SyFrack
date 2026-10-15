use std::sync::OnceLock;

pub struct BlueNoise {
    pub size: usize,
    /// Rank / (size²), in [0, 1), toroidal.
    pub values: Vec<f32>,
}

impl BlueNoise {
    #[inline]
    pub fn at(&self, x: u32, y: u32) -> f32 {
        let n = self.size as u32;
        self.values[((y % n) * n + (x % n)) as usize]
    }
}

/// Shared 64 × 64 mask, generated once.
pub fn blue_noise() -> &'static BlueNoise {
    static MASK: OnceLock<BlueNoise> = OnceLock::new();
    MASK.get_or_init(|| BlueNoise {
        size: 64,
        values: generate(64, 0x5EEDB10E),
    })
}

fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28).wrapping_add(4))) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// Toroidal Gaussian energy field over a binary pattern.
struct Field {
    n: usize,
    kernel: Vec<f32>,
    energy: Vec<f32>,
    on: Vec<bool>,
}

impl Field {
    fn new(n: usize) -> Self {
        let sigma = 1.5f32;
        let mut kernel = vec![0.0; n * n];
        for y in 0..n {
            for x in 0..n {
                let dx = x.min(n - x) as f32;
                let dy = y.min(n - y) as f32;
                kernel[y * n + x] = (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
            }
        }
        Self {
            n,
            kernel,
            energy: vec![0.0; n * n],
            on: vec![false; n * n],
        }
    }

    fn set(&mut self, i: usize, value: bool) {
        if self.on[i] == value {
            return;
        }
        self.on[i] = value;
        let sign = if value { 1.0 } else { -1.0 };
        let n = self.n;
        let (ix, iy) = (i % n, i / n);
        for y in 0..n {
            let dy = (y + n - iy) % n;
            for x in 0..n {
                let dx = (x + n - ix) % n;
                self.energy[y * n + x] += sign * self.kernel[dy * n + dx];
            }
        }
    }

    /// Index of the "on" pixel with the highest energy (tightest cluster).
    fn tightest_cluster(&self) -> usize {
        (0..self.n * self.n)
            .filter(|i| self.on[*i])
            .max_by(|a, b| self.energy[*a].total_cmp(&self.energy[*b]).then(b.cmp(a)))
            .expect("no pixel on")
    }

    /// Index of the "off" pixel with the lowest energy (largest void).
    fn largest_void(&self) -> usize {
        (0..self.n * self.n)
            .filter(|i| !self.on[*i])
            .min_by(|a, b| self.energy[*a].total_cmp(&self.energy[*b]).then(a.cmp(b)))
            .expect("no pixel off")
    }
}

/// Generates an n × n blue-noise rank mask (void-and-cluster). Returns rank / n².
pub fn generate(n: usize, seed: u64) -> Vec<f32> {
    let total = n * n;
    let initial = (total / 10).max(1);
    let mut field = Field::new(n);

    // Random initial pattern.
    let mut rng = pcg(seed as u32 ^ pcg((seed >> 32) as u32));
    let mut count = 0;
    while count < initial {
        rng = pcg(rng);
        let i = (rng as usize) % total;
        if !field.on[i] {
            field.set(i, true);
            count += 1;
        }
    }
    // Relax: move tightest cluster into largest void until stable.
    loop {
        let c = field.tightest_cluster();
        field.set(c, false);
        let v = field.largest_void();
        field.set(v, true);
        if v == c {
            break;
        }
    }
    let prototype = field.on.clone();
    let mut ranks = vec![0usize; total];

    // Phase 1: remove clusters, ranks initial-1 .. 0.
    let mut rank = initial;
    while rank > 0 {
        let c = field.tightest_cluster();
        rank -= 1;
        ranks[c] = rank;
        field.set(c, false);
    }
    // Phase 2: fill voids, ranks initial .. total/2.
    let mut field = Field::new(n);
    for (i, on) in prototype.iter().enumerate() {
        if *on {
            field.set(i, true);
        }
    }
    let mut rank = initial;
    while rank < total / 2 {
        let v = field.largest_void();
        ranks[v] = rank;
        rank += 1;
        field.set(v, true);
    }
    // Phase 3: treat remaining "off" pixels as the pattern; remove tightest clusters.
    let mut inverted = Field::new(n);
    for i in 0..total {
        if !field.on[i] {
            inverted.set(i, true);
        }
    }
    while rank < total {
        let c = inverted.tightest_cluster();
        ranks[c] = rank;
        rank += 1;
        inverted.set(c, false);
    }
    ranks.iter().map(|r| *r as f32 / total as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_is_a_permutation_of_ranks() {
        let n = 16;
        let v = generate(n, 1);
        assert_eq!(v.len(), n * n);
        let mut ranks: Vec<usize> = v
            .iter()
            .map(|x| (x * (n * n) as f32).round() as usize)
            .collect();
        ranks.sort_unstable();
        assert_eq!(ranks, (0..n * n).collect::<Vec<_>>());
    }

    #[test]
    fn mask_is_blue_not_white() {
        // In a blue-noise mask, 4-neighbours of a low-rank pixel should rarely
        // also be low-rank. Count violations among the darkest 10 %.
        let n = 32;
        let v = generate(n, 5);
        let at = |x: usize, y: usize| v[(y % n) * n + (x % n)];
        let mut low_pairs = 0;
        for y in 0..n {
            for x in 0..n {
                if at(x, y) < 0.1 {
                    if at(x + 1, y) < 0.1 {
                        low_pairs += 1;
                    }
                    if at(x, y + 1) < 0.1 {
                        low_pairs += 1;
                    }
                }
            }
        }
        // White noise would give about 2 * 1024 * 0.1 * 0.1 = 20 pairs.
        assert!(low_pairs <= 4, "{low_pairs} adjacent low pairs");
    }

    #[test]
    fn shared_mask_is_64_and_tiles() {
        let bn = blue_noise();
        assert_eq!(bn.size, 64);
        assert_eq!(bn.at(0, 0), bn.at(64, 64));
    }
}
