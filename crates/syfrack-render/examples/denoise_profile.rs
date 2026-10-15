//! Deterministic CPU export benchmark; optional threads via RAYON_NUM_THREADS.
use std::time::Instant;
use syfrack_core::HdrImage;

fn main() {
    let width = 1280;
    let height = 720;
    let mut data = Vec::new();
    let mut depth = Vec::new();
    let mut normals = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let noise = ((x * 73 + y * 31) % 23) as f32 / 22.0;
            let base = if x < width / 2 { 0.2 } else { 0.8 };
            data.push([base + noise * 0.2, base, base * 0.5, 1.0]);
            depth.push(if y < 40 { -1.0 } else { 4.0 + y as f32 * 0.001 });
            normals.push(if y < 40 {
                [0.0; 4]
            } else {
                [0.0, 1.0, 0.0, 1.0]
            });
        }
    }
    let image = HdrImage::from_data(width, height, data);
    for trial in 0..4 {
        let start = Instant::now();
        let result = syfrack_render::denoise::denoise(&image, &depth, &normals);
        let elapsed = start.elapsed();
        let hash = result
            .data
            .iter()
            .flatten()
            .fold(0xcbf29ce484222325u64, |h, v| {
                (h ^ v.to_bits() as u64).wrapping_mul(0x100000001b3)
            });
        println!(
            "trial={trial} size={width}x{height} ms={:.3} hash={hash:016x}",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
