use glam::{Vec3, Vec4};
use syfrack_core::scene::phenomena::FieldDomain;

#[derive(Clone, Debug)]
pub struct Grid {
    pub domain: FieldDomain,
    pub data: Vec<Vec4>,
}
impl Grid {
    pub fn zero(domain: &FieldDomain) -> Self {
        Self {
            domain: domain.clone(),
            data: vec![Vec4::ZERO; domain.cells() as usize],
        }
    }
    pub fn index(&self, x: u32, y: u32, z: u32) -> usize {
        ((z * self.domain.resolution[1] + y) * self.domain.resolution[0] + x) as usize
    }
    pub fn coord(&self, index: usize) -> [u32; 3] {
        let [nx, ny, _] = self.domain.resolution;
        let i = index as u32;
        [i % nx, (i / nx) % ny, i / (nx * ny)]
    }
    pub fn spacing(&self) -> Vec3 {
        let r = self.domain.resolution;
        self.domain.half * 2.0
            / Vec3::new(
                r[0].saturating_sub(1).max(1) as f32,
                r[1].saturating_sub(1).max(1) as f32,
                r[2].saturating_sub(1).max(1) as f32,
            )
    }
    pub fn point(&self, index: usize) -> Vec3 {
        let q = self.coord(index);
        let mut p = Vec3::from_array(q.map(|x| x as f32)) * self.spacing() + self.domain.center
            - self.domain.half;
        for a in 0..3 {
            if self.domain.resolution[a] == 1 {
                p[a] = self.domain.center[a];
            }
        }
        p
    }
    pub fn contains(&self, p: Vec3) -> bool {
        (p - self.domain.center).abs().cmple(self.domain.half).all()
    }
    pub fn sample(&self, p: Vec3) -> Vec4 {
        let r = self.domain.resolution;
        let u = ((p - self.domain.center + self.domain.half) / (self.domain.half * 2.0))
            .clamp(Vec3::ZERO, Vec3::ONE);
        let q = u * Vec3::new((r[0] - 1) as f32, (r[1] - 1) as f32, (r[2] - 1) as f32);
        let low = q.floor();
        let f = q - low;
        let l = low.to_array().map(|v| v as u32);
        let h = [
            (l[0] + 1).min(r[0] - 1),
            (l[1] + 1).min(r[1] - 1),
            (l[2] + 1).min(r[2] - 1),
        ];
        let mut value = Vec4::ZERO;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let ix = if x == 0 { l[0] } else { h[0] };
                    let iy = if y == 0 { l[1] } else { h[1] };
                    let iz = if z == 0 { l[2] } else { h[2] };
                    let weight = if x == 0 { 1.0 - f.x } else { f.x }
                        * if y == 0 { 1.0 - f.y } else { f.y }
                        * if z == 0 { 1.0 - f.z } else { f.z };
                    value += self.data[self.index(ix, iy, iz)] * weight;
                }
            }
        }
        value
    }
    pub fn neighbor(&self, index: usize, axis: usize, delta: i32, periodic: bool) -> usize {
        let mut c = self.coord(index);
        let n = self.domain.resolution[axis] as i32;
        let v = c[axis] as i32 + delta;
        c[axis] = if periodic {
            v.rem_euclid(n)
        } else {
            v.clamp(0, n - 1)
        } as u32;
        self.index(c[0], c[1], c[2])
    }
    pub fn laplacian(&self, index: usize, periodic: bool) -> Vec4 {
        let h = self.spacing();
        let mut sum = Vec4::ZERO;
        for a in 0..3 {
            if self.domain.resolution[a] > 1 {
                sum += (self.data[self.neighbor(index, a, -1, periodic)]
                    + self.data[self.neighbor(index, a, 1, periodic)]
                    - self.data[index] * 2.0)
                    / (h[a] * h[a]);
            }
        }
        sum
    }
    /// A bound for every trilinear interpolant derivative, not a sampled gradient.
    pub fn lipschitz(&self, channel: usize) -> f32 {
        let h = self.spacing();
        let mut maxima = Vec3::ZERO;
        for i in 0..self.data.len() {
            let c = self.coord(i);
            for a in 0..3 {
                if c[a] + 1 < self.domain.resolution[a] {
                    let j = self.neighbor(i, a, 1, false);
                    maxima[a] =
                        maxima[a].max((self.data[j][channel] - self.data[i][channel]).abs() / h[a]);
                }
            }
        }
        maxima.length()
    }
}
