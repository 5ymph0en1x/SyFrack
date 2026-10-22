//! Warm, end-to-end comparison of GPU-only, CPU-only and simultaneous hybrid
//! exports. The CPU/GPU baselines are measurements, not alternative defaults.
pub use crate::execution_profile::{DeviceDispatchTiming, DispatchTiming};
use crate::{ExportMode, ExportSession, RenderConfig, Renderer};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use syfrack_core::{HdrImage, Scene};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ImageTolerance {
    /// Allowed linear-HDR RMSE floor, plus `relative_rmse * reference_rms`.
    pub absolute_rmse: f64,
    pub relative_rmse: f64,
}
impl Default for ImageTolerance {
    fn default() -> Self {
        Self {
            absolute_rmse: 1e-4,
            relative_rmse: 0.01,
        }
    }
}
impl ImageTolerance {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.absolute_rmse.is_finite()
                && self.absolute_rmse >= 0.0
                && self.relative_rmse.is_finite()
                && (0.0..=1.0).contains(&self.relative_rmse),
            "invalid HDR tolerance"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageDifference {
    pub rmse: f64,
    pub reference_rms: f64,
    pub relative_rmse: f64,
    pub mean_absolute_error: f64,
    pub max_absolute_error: f64,
    pub alpha_rmse: f64,
    pub differing_rgb_bits: usize,
    pub worst_pixel: [u32; 2],
}
impl ImageDifference {
    pub fn require(&self, tolerance: ImageTolerance) -> Result<()> {
        tolerance.validate()?;
        ensure!(self.rmse <= tolerance.absolute_rmse + tolerance.relative_rmse * self.reference_rms
            && self.alpha_rmse <= tolerance.absolute_rmse + tolerance.relative_rmse,
            "native CPU/GPU quality tolerance exceeded: HDR RMSE {}, reference RMS {}, alpha RMSE {}",
            self.rmse, self.reference_rms, self.alpha_rmse);
        Ok(())
    }
}

pub fn compare(reference: &HdrImage, candidate: &HdrImage) -> Result<ImageDifference> {
    ensure!(
        reference.width > 0
            && reference.height > 0
            && (reference.width, reference.height) == (candidate.width, candidate.height)
            && reference.data.len() == reference.width as usize * reference.height as usize
            && candidate.data.len() == reference.data.len(),
        "image dimensions differ or are invalid"
    );
    let (mut squares, mut energy, mut absolute, mut alpha, mut max, mut bits, mut worst) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize, [0, 0]);
    for (i, (a, b)) in reference.data.iter().zip(&candidate.data).enumerate() {
        ensure!(
            a.iter().chain(b).all(|v| v.is_finite()),
            "nonfinite HDR at pixel {i}"
        );
        for c in 0..3 {
            let d = a[c] as f64 - b[c] as f64;
            squares += d * d;
            energy += (a[c] as f64).powi(2);
            absolute += d.abs();
            bits += usize::from(a[c].to_bits() != b[c].to_bits());
            if d.abs() > max {
                max = d.abs();
                worst = [
                    (i % reference.width as usize) as u32,
                    (i / reference.width as usize) as u32,
                ];
            }
        }
        alpha += (a[3] as f64 - b[3] as f64).powi(2);
    }
    let n = reference.data.len() as f64 * 3.0;
    let rmse = (squares / n).sqrt();
    let reference_rms = (energy / n).sqrt();
    Ok(ImageDifference {
        rmse,
        reference_rms,
        relative_rmse: rmse / reference_rms.max(1e-12),
        mean_absolute_error: absolute / n,
        max_absolute_error: max,
        alpha_rmse: (alpha / reference.data.len() as f64).sqrt(),
        differing_rgb_bits: bits,
        worst_pixel: worst,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkStatistics {
    pub producers: Vec<String>,
    pub cpu_tiles: usize,
    pub gpu_tiles: usize,
    pub tile_size: u32,
    /// Host-observed delivery of each device's last tile, including readback.
    pub cpu_last_tile_ms: f64,
    pub gpu_last_tile_ms: f64,
    /// Host submission-to-completion intervals, excluding warm-up and readback.
    /// Their overlap diagnoses scheduling; it is not GPU hardware utilization.
    #[serde(default)]
    pub dispatch_timing: DispatchTiming,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(default)]
    pub repetition: u32,
    pub backend: ExportMode,
    pub seconds: f64,
    pub pixel_samples_per_second: f64,
    pub workers: WorkStatistics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HybridBenchmark {
    pub version: u32,
    pub scene: Scene,
    pub config: RenderConfig,
    pub tolerance: ImageTolerance,
    pub measurements: Vec<Measurement>,
    pub hybrid_speedup_over_gpu: f64,
    pub ideal_combined_speedup: f64,
    pub cpu_difference: ImageDifference,
    pub hybrid_difference: ImageDifference,
}

#[derive(Clone, Copy, Debug)]
pub enum BenchmarkPhase {
    Warmup(ExportMode),
    Measurement {
        backend: ExportMode,
        repetition: u32,
    },
}
impl HybridBenchmark {
    /// A hybrid performance claim must exercise both workers in overlapping
    /// submission/completion intervals. Counts alone can also describe a serial run.
    pub fn require_concurrent_work(&self) -> Result<()> {
        ensure!(
            self.measurements
                .iter()
                .any(|m| m.backend == ExportMode::Hybrid),
            "benchmark contains no hybrid measurement"
        );
        for m in self
            .measurements
            .iter()
            .filter(|m| m.backend == ExportMode::Hybrid)
        {
            ensure!(m.workers.cpu_tiles > 0 && m.workers.gpu_tiles > 0 && m.workers.dispatch_timing.overlap_wall_ms > 0.0,
                "hybrid run {} did not exercise overlapping CPU and GPU dispatches; inspect its timing report or increase the workload", m.repetition + 1);
        }
        Ok(())
    }

    /// Qualify the assembled hybrid image. The full CPU image is a diagnostic
    /// baseline and retains its own error metrics, even when it exceeds this
    /// threshold. It must not be confused with the mixed export's error.
    pub fn require_quality(&self) -> Result<()> {
        self.hybrid_difference.require(self.tolerance)
    }
}

fn run(
    renderer: &Renderer,
    scene: &Scene,
    cfg: &RenderConfig,
    mode: ExportMode,
) -> Result<(Measurement, HdrImage)> {
    let started = Instant::now();
    let mut session = if mode == ExportMode::Cpu {
        ExportSession::cpu(scene, cfg)?
    } else {
        ExportSession::new(renderer.clone(), scene, cfg, mode)?
    };
    while !session.step(Duration::from_millis(20))? {}
    session.stop_workers();
    let seconds = started.elapsed().as_secs_f64();
    let ExportSession::Tiled(tiled) = &session else {
        unreachable!()
    };
    let workers = tiled.work_statistics();
    let image = session.readback()?.color;
    Ok((
        Measurement {
            repetition: 0,
            backend: mode,
            seconds,
            pixel_samples_per_second: cfg.width as f64
                * cfg.height as f64
                * cfg.samples_per_pixel as f64
                / seconds,
            workers,
        },
        image,
    ))
}

/// Warm the same pipelines once before measuring. Each timed run includes tile
/// pools, work scheduling and readbacks, excluding image encoding. No claim of
/// speedup is accepted from tile counts alone; the report contains wall times.
pub fn benchmark(
    renderer: &Renderer,
    scene: &Scene,
    cfg: &RenderConfig,
    tolerance: ImageTolerance,
    progress: &mut dyn FnMut(&Measurement),
) -> Result<HybridBenchmark> {
    benchmark_repeated(renderer, scene, cfg, tolerance, 1, progress)
}

/// Repeat interleaved baselines in alternating order to expose timing noise
/// and sustained CPU/GPU contention. Report median speedups and worst image
/// errors across all repetitions, rather than selecting the fastest run.
pub fn benchmark_repeated(
    renderer: &Renderer,
    scene: &Scene,
    cfg: &RenderConfig,
    tolerance: ImageTolerance,
    repetitions: u32,
    progress: &mut dyn FnMut(&Measurement),
) -> Result<HybridBenchmark> {
    benchmark_repeated_with_phases(
        renderer,
        scene,
        cfg,
        tolerance,
        repetitions,
        progress,
        &mut |_| {},
    )
}

/// The phase callback runs before warm-up or a timed measurement starts, so a
/// caller can distinguish CPU-only JIT preparation from simultaneous rendering.
pub fn benchmark_repeated_with_phases(
    renderer: &Renderer,
    scene: &Scene,
    cfg: &RenderConfig,
    tolerance: ImageTolerance,
    repetitions: u32,
    progress: &mut dyn FnMut(&Measurement),
    phase: &mut dyn FnMut(BenchmarkPhase),
) -> Result<HybridBenchmark> {
    tolerance.validate()?;
    ensure!(
        (1..=20).contains(&repetitions),
        "repetitions must be in 1..=20"
    );
    ensure!(
        renderer.context().info.device_type != wgpu::DeviceType::Cpu,
        "hybrid benchmark needs a hardware GPU and a distinct software CPU adapter"
    );
    let mut warm = cfg.clone();
    warm.width = cfg.width.min(16);
    warm.height = cfg.height.min(16);
    warm.samples_per_pixel = 1;
    warm.tile_size = 16;
    for backend in [ExportMode::Tiled, ExportMode::Cpu] {
        phase(BenchmarkPhase::Warmup(backend));
        run(renderer, scene, &warm, backend)?;
    }
    let mut measurements = Vec::new();
    let mut cpu_differences = Vec::new();
    let mut hybrid_differences = Vec::new();
    for repetition in 0..repetitions {
        let modes = if repetition % 2 == 0 {
            [ExportMode::Tiled, ExportMode::Cpu, ExportMode::Hybrid]
        } else {
            [ExportMode::Hybrid, ExportMode::Cpu, ExportMode::Tiled]
        };
        let mut images = Vec::new();
        for mode in modes {
            phase(BenchmarkPhase::Measurement {
                backend: mode,
                repetition,
            });
            let (mut measurement, image) = run(renderer, scene, cfg, mode)?;
            measurement.repetition = repetition;
            progress(&measurement);
            measurements.push(measurement);
            images.push((mode, image));
        }
        let image = |mode| &images.iter().find(|(m, _)| *m == mode).unwrap().1;
        cpu_differences.push(compare(image(ExportMode::Tiled), image(ExportMode::Cpu))?);
        hybrid_differences.push(compare(
            image(ExportMode::Tiled),
            image(ExportMode::Hybrid),
        )?);
    }
    let median = |mode| {
        let mut values: Vec<_> = measurements
            .iter()
            .filter(|m| m.backend == mode)
            .map(|m| m.seconds)
            .collect();
        values.sort_by(f64::total_cmp);
        (values[(values.len() - 1) / 2] + values[values.len() / 2]) * 0.5
    };
    let speedup = median(ExportMode::Tiled) / median(ExportMode::Hybrid);
    let ideal = 1.0 + median(ExportMode::Tiled) / median(ExportMode::Cpu);
    let worst = |differences: Vec<ImageDifference>| {
        let alpha = differences.iter().map(|d| d.alpha_rmse).fold(0.0, f64::max);
        let mut worst = differences
            .into_iter()
            .max_by(|a, b| a.rmse.total_cmp(&b.rmse))
            .unwrap();
        worst.alpha_rmse = alpha;
        worst
    };
    Ok(HybridBenchmark {
        version: 3,
        scene: scene.clone(),
        config: cfg.clone(),
        tolerance,
        measurements,
        hybrid_speedup_over_gpu: speedup,
        ideal_combined_speedup: ideal,
        cpu_difference: worst(cpu_differences),
        hybrid_difference: worst(hybrid_differences),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tolerance_preserves_hdr_has_an_absolute_floor_and_checks_alpha() {
        let reference = HdrImage::from_data(1, 1, vec![[10.0, 2.0, 1.0, 1.0]]);
        let mut candidate = reference.clone();
        candidate.data[0][0] += 0.001;
        compare(&reference, &candidate)
            .unwrap()
            .require(ImageTolerance::default())
            .unwrap();
        candidate.data[0][0] += 1.0;
        assert!(compare(&reference, &candidate)
            .unwrap()
            .require(ImageTolerance::default())
            .is_err());
        candidate = reference.clone();
        candidate.data[0][3] = 0.0;
        assert!(compare(&reference, &candidate)
            .unwrap()
            .require(ImageTolerance::default())
            .is_err());
        candidate.data[0][0] = f32::NAN;
        assert!(compare(&reference, &candidate).is_err());
        let black = HdrImage::from_data(1, 1, vec![[0.0, 0.0, 0.0, 1.0]]);
        let near_black = HdrImage::from_data(1, 1, vec![[0.00001, 0.0, 0.0, 1.0]]);
        compare(&black, &near_black)
            .unwrap()
            .require(ImageTolerance::default())
            .unwrap();
        assert!(ImageTolerance {
            relative_rmse: f64::NAN,
            absolute_rmse: 0.0
        }
        .validate()
        .is_err());
    }
}
