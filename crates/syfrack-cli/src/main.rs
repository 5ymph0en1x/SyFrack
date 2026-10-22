use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use syfrack_color::{develop_and_encode, read_exr, write_exr, write_png};
use syfrack_core::{clay_scene, thumbnail_scene, FractalTexture, Graph, Look, Scene};
use syfrack_render::{ExportMode, ExportSession, GpuContext, RenderConfig, Renderer, SessionState};
mod mosaic;
mod nature;

#[derive(Parser)]
#[command(name = "syfrack", version, about = "SyFrack fractal renderer")]
struct Cli {
    /// Language for localized scene diagnostics (en or fr).
    #[arg(long, global = true, value_parser = ["en", "fr"], default_value = "en")]
    lang: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Reinterpret an image as an adaptive mosaic of coloured fractals (CPU).
    Mosaic {
        #[command(subcommand)]
        cmd: mosaic::MosaicCommand,
    },
    /// Coupled natural fields, frozen simulations, animation and variation branches.
    Nature {
        #[command(subcommand)]
        cmd: nature::NatureCommand,
    },
    /// Reports the GPU adapter and runs a trivial compute shader.
    Probe {
        /// Probe the software Vulkan CPU adapter without compiling a scene.
        #[arg(long)]
        cpu: bool,
    },
    /// Measure GPU-only, CPU-only and simultaneous hybrid export throughput.
    BenchmarkHybrid {
        scene: PathBuf,
        #[arg(long, default_value_t = 3, value_parser=clap::value_parser!(u32).range(1..=20))]
        repeats: u32,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 128, value_parser=clap::value_parser!(u32).range(8..=4096))]
        width: u32,
        #[arg(long, default_value_t = 96, value_parser=clap::value_parser!(u32).range(8..=4096))]
        height: u32,
        #[arg(long, default_value_t = 8, value_parser=clap::value_parser!(u32).range(1..=4096))]
        samples: u32,
        #[arg(long, default_value_t = 1)]
        bounces: u32,
        #[arg(long, default_value_t = 128)]
        steps: u32,
        #[arg(long, default_value_t = 32)]
        tile: u32,
        #[arg(long, default_value_t = 0.01)]
        relative_rmse: f64,
        #[arg(long, default_value_t = 0.0001)]
        absolute_rmse: f64,
    },
    /// Strict bitwise CPU/WGSL audit of all eleven bundled distance graphs.
    Parity {
        #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u32).range(16..=1_000_000))]
        points: u32,
        #[arg(long)]
        out: PathBuf,
        /// Audit a software Vulkan adapter instead of the hardware GPU.
        #[arg(long)]
        cpu: bool,
        /// Save measurements without requiring the strict parity gate to pass.
        #[arg(long)]
        report_only: bool,
    },
    /// Writes a built-in example scene.
    ExampleScene {
        #[arg(long)]
        out: PathBuf,
        /// A bundled form, e.g. mandelbulb, dentelle-inversee, menger-torsade, or flame.
        #[arg(long, default_value = "sphere")]
        kind: String,
        /// Apply an independent fractal texture from this bundled graph.
        #[arg(long)]
        texture: Option<String>,
    },
    /// Renders a scene to <out>.exr, <out>.png and <out>.ron.
    Render {
        scene: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        #[arg(long, default_value_t = 256)]
        samples: u32,
        #[arg(long, default_value_t = 4)]
        bounces: u32,
        /// Maximum tracing steps per ray; raise for fine, layered architecture.
        #[arg(long, default_value_t = 256)]
        steps: u32,
        /// Primary geometry tolerance in pixels (0.125..1); 0.25 for fine forms.
        #[arg(long, default_value_t = 1.0)]
        pixel_tolerance: f32,
        /// Reduce PNG noise using first-hit normals and depth; keep the raw EXR.
        #[arg(long)]
        denoise: bool,
        /// Learn a bounded directional GI proposal for longer still renders.
        #[arg(long)]
        guide: bool,
        #[arg(long, default_value_t = 256)]
        tile: u32,
        /// auto: GPU-only, using wavefront for new 3D exports.
        #[arg(long, default_value = "auto", value_parser = ["auto", "gpu", "tiled", "wavefront", "hybrid", "cpu"])]
        backend: String,
        /// GPU dispatch latency target in ms; longer batches suit final exports.
        #[arg(long, default_value_t=800, value_parser=clap::value_parser!(u32).range(50..=800))]
        dispatch_ms: u32,
        /// Stop after this many seconds and keep what has converged.
        #[arg(long, allow_negative_numbers = true)]
        time_budget: Option<f64>,
        /// Directory for pause/resume state (default <out>.state when checkpoints are requested).
        #[arg(long)]
        state: Option<PathBuf>,
        /// Save the state every N seconds (and when a time budget interrupts).
        #[arg(long, allow_negative_numbers = true)]
        checkpoint_every: Option<f64>,
        /// Write <out>.preview.png every N seconds.
        #[arg(long, allow_negative_numbers = true)]
        preview_every: Option<f64>,
        /// Pause safely when this file appears, saving outputs and the checkpoint.
        #[arg(long)]
        pause_file: Option<PathBuf>,
    },
    /// Continues a render from a state directory.
    Resume {
        state: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Explicitly switch a tiled checkpoint's backend, accepting native float differences.
        #[arg(long, value_parser = ["auto", "gpu", "tiled", "wavefront", "hybrid", "cpu"])]
        backend: Option<String>,
        /// Override scheduling latency without changing saved samples or shading.
        #[arg(long, value_parser=clap::value_parser!(u32).range(50..=800))]
        dispatch_ms: Option<u32>,
        #[arg(long, allow_negative_numbers = true)]
        checkpoint_every: Option<f64>,
        #[arg(long, allow_negative_numbers = true)]
        preview_every: Option<f64>,
        #[arg(long, allow_negative_numbers = true)]
        time_budget: Option<f64>,
        #[arg(long)]
        pause_file: Option<PathBuf>,
    },
    /// Prints the progress stored in a state directory.
    Info { state: PathBuf },
    /// Re-develops a linear EXR with the look of a scene (or a bare look file).
    Grade {
        exr: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, conflicts_with = "look")]
        scene: Option<PathBuf>,
        #[arg(long)]
        look: Option<PathBuf>,
        #[arg(long, default_value_t = 16)]
        bits: u8,
    },
    /// Form-graph tools.
    Graph {
        #[command(subcommand)]
        cmd: GraphCmd,
    },
}

#[derive(Subcommand)]
enum GraphCmd {
    /// Writes the readable Forme language for a scene or graph.
    Language {
        path: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Compiles a Forme source into a scene; optionally keeps an existing scene's appearance.
    Build {
        path: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        scene: Option<PathBuf>,
    },
    /// Validates the form graph of a scene or of a bare graph file and prints a summary.
    Validate { path: PathBuf },
    /// Writes the WGSL emitted for the graph.
    Compile {
        path: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Rewrites a scene in the current format (comments dropped); keeps <path>.bak.
    /// A scene older than plan 4 (hand-written formulas) is refused.
    Upgrade {
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Renders a 320x180 clay thumbnail next to the file (<stem>.png).
    Thumbnail {
        path: PathBuf,
        #[arg(long, default_value_t = 32)]
        samples: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    syfrack_i18n::set_language(syfrack_i18n::Language::parse(&cli.lang).unwrap());
    match cli.command {
        Command::Mosaic { cmd } => mosaic::run(cmd),
        Command::Nature { cmd } => nature::run(cmd),
        Command::Probe { cpu } => probe(cpu),
        Command::BenchmarkHybrid {
            scene,
            repeats,
            out,
            width,
            height,
            samples,
            bounces,
            steps,
            tile,
            relative_rmse,
            absolute_rmse,
        } => {
            let tolerance = syfrack_render::hybrid_benchmark::ImageTolerance {
                relative_rmse,
                absolute_rmse,
            };
            tolerance.validate()?;
            let scene = Scene::load(&scene)?;
            let mut cfg = RenderConfig::new(width, height, samples);
            cfg.max_bounces = bounces;
            cfg.max_steps = steps;
            cfg.tile_size = tile;
            cfg.dispatch_budget_ms = 800;
            eprintln!(
                "Warming GPU and CPU pipelines before timing {repeats} interleaved repetitions..."
            );
            let report = syfrack_render::hybrid_benchmark::benchmark_repeated_with_phases(
                &gpu_renderer()?,
                &scene,
                &cfg,
                tolerance,
                repeats,
                &mut |m| {
                    println!(
                        "run {} {:?}: {:.3} s, {:.0} pixel-samples/s, CPU {} tiles, GPU {} tiles",
                        m.repetition + 1,
                        m.backend,
                        m.seconds,
                        m.pixel_samples_per_second,
                        m.workers.cpu_tiles,
                        m.workers.gpu_tiles
                    );
                    if m.backend == syfrack_render::ExportMode::Hybrid {
                        let t = &m.workers.dispatch_timing;
                        let interval =
                            |d: &syfrack_render::hybrid_benchmark::DeviceDispatchTiming| match (
                                d.first_ms, d.last_ms,
                            ) {
                                (Some(first), Some(last)) => format!("{first:.1} .. {last:.1} ms"),
                                _ => "inactive".into(),
                            };
                        println!(
                            "  CPU/GPU in-flight dispatch overlap (host clock): {:.1} ms; CPU dispatches {}, GPU dispatches {}",
                            t.overlap_wall_ms, t.cpu.dispatches, t.gpu.dispatches
                        );
                        println!(
                            "  CPU timeline: {}; GPU timeline: {}",
                            interval(&t.cpu),
                            interval(&t.gpu)
                        );
                    }
                },
                &mut |phase| {
                    use syfrack_render::hybrid_benchmark::BenchmarkPhase;
                    match phase {
                        BenchmarkPhase::Warmup(backend) => {
                            eprintln!("Warm-up {backend:?} only (excluded from timing)...")
                        }
                        BenchmarkPhase::Measurement {
                            backend,
                            repetition,
                        } => eprintln!(
                            "Starting run {} {backend:?}{}...",
                            repetition + 1,
                            if backend == syfrack_render::ExportMode::Hybrid {
                                " (CPU + hardware GPU concurrently)"
                            } else {
                                " only (baseline)"
                            }
                        ),
                    }
                },
            )?;
            std::fs::write(
                &out,
                ron::ser::to_string_pretty(&report, ron::ser::PrettyConfig::new())?,
            )?;
            println!(
                "Hybrid speedup / GPU: {:.3}x; ideal sum of isolated throughputs: {:.3}x",
                report.hybrid_speedup_over_gpu, report.ideal_combined_speedup
            );
            println!(
                "HDR RMSE CPU {:.6e}, hybrid {:.6e}; report: {}",
                report.cpu_difference.rmse,
                report.hybrid_difference.rmse,
                out.display()
            );
            println!(
                "CPU-only diagnostic within tolerance: {}; assembled hybrid within tolerance: {}",
                report.cpu_difference.require(tolerance).is_ok(),
                report.hybrid_difference.require(tolerance).is_ok()
            );
            report.require_concurrent_work()?;
            report.require_quality()
        }
        Command::Parity {
            points,
            out,
            cpu,
            report_only,
        } => {
            let context = Arc::new(if cpu {
                GpuContext::new_cpu()?
            } else {
                GpuContext::new()?
            });
            let renderer = Renderer::new(context)?;
            let report = syfrack_render::parity::audit(&renderer, points as usize, 11, &mut |g| {
                println!(
                    "{}: distance {}/{}, traps {:?}, iterations {}, material {}, max ULP {}",
                    g.graph,
                    g.distance_mismatches,
                    g.points,
                    g.trap_mismatches,
                    g.iteration_mismatches,
                    g.material_mismatches,
                    g.max_distance_ulp
                );
            })?;
            for r in &report.arithmetic {
                println!(
                    "math {}: {}/{} bit differences",
                    r.operation, r.mismatches, r.inputs
                );
            }
            std::fs::write(
                &out,
                ron::ser::to_string_pretty(&report, ron::ser::PrettyConfig::new())?,
            )?;
            println!("parity report: {}", out.display());
            if !report_only {
                report.require_bit_exact()?;
            }
            Ok(())
        }
        Command::ExampleScene { out, kind, texture } => {
            let mut scene = Scene::example_for(&kind.to_ascii_lowercase())?;
            if let Some(name) = texture {
                let graph = Scene::example_for(&name.to_ascii_lowercase())?
                    .fractal
                    .to_graph();
                scene.material.texture = Some(FractalTexture {
                    graph,
                    ..Default::default()
                });
            }
            scene
                .save(&out)
                .with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
            Ok(())
        }
        Command::Render {
            scene,
            out,
            width,
            height,
            samples,
            bounces,
            steps,
            pixel_tolerance,
            denoise,
            guide,
            tile,
            dispatch_ms,
            backend,
            time_budget,
            state,
            checkpoint_every,
            preview_every,
            pause_file,
        } => {
            let time_budget = seconds_arg("time-budget", time_budget)?;
            let checkpoint_every = seconds_arg("checkpoint-every", checkpoint_every)?;
            let preview_every = seconds_arg("preview-every", preview_every)?;
            for (name, value) in [
                ("width", width),
                ("height", height),
                ("samples", samples),
                ("steps", steps),
            ] {
                if value < 1 {
                    anyhow::bail!("--{name} must be at least 1, got {value}");
                }
            }
            let mut cfg = RenderConfig::new(width, height, samples);
            cfg.max_bounces = bounces;
            cfg.max_steps = steps;
            cfg.pixel_tolerance = pixel_tolerance;
            cfg.denoise = denoise;
            cfg.guiding = guide;
            cfg.tile_size = tile;
            cfg.dispatch_budget_ms = dispatch_ms;
            // The budget is enforced by `drive`, not by the session.
            cfg.time_budget = None;
            let wants_state = state.is_some() || checkpoint_every.is_some() || pause_file.is_some();
            let state_dir = if wants_state {
                Some(state.unwrap_or_else(|| with_suffix(&stem(&out), "state")))
            } else {
                None
            };
            if let Some(dir) = &state_dir {
                check_out_vs_state(&out, dir)?;
                check_state_dir_free(dir)?;
            }
            let scene_value =
                Scene::load(&scene).with_context(|| format!("loading {}", scene.display()))?;
            scene_value.validate()?;
            let checkpoint_every =
                checkpoint_every.or(state_dir.as_ref().map(|_| Duration::from_secs(60)));
            let mode = match backend.as_str() {
                "gpu" => ExportMode::Gpu,
                "tiled" => ExportMode::Tiled,
                "wavefront" => ExportMode::Wavefront,
                "hybrid" => ExportMode::Hybrid,
                "cpu" => ExportMode::Cpu,
                _ => ExportMode::Auto,
            };
            let session = if mode == ExportMode::Cpu {
                ExportSession::cpu(&scene_value, &cfg)?
            } else {
                ExportSession::new(gpu_renderer()?, &scene_value, &cfg, mode)?
            };
            drive(
                session,
                &Drive {
                    out,
                    state_dir,
                    checkpoint_every,
                    preview_every,
                    time_budget,
                    pause_file,
                },
            )
        }
        Command::Resume {
            state,
            out,
            backend,
            dispatch_ms,
            checkpoint_every,
            preview_every,
            time_budget,
            pause_file,
        } => {
            let checkpoint_every = seconds_arg("checkpoint-every", checkpoint_every)?;
            let preview_every = seconds_arg("preview-every", preview_every)?;
            let time_budget = seconds_arg("time-budget", time_budget)?;
            check_out_vs_state(&out, &state)?;
            let resumed = match backend.as_deref() {
                Some("cpu") => ExportSession::resume_cpu(&state),
                Some(name) => ExportSession::resume_with_backend(
                    gpu_renderer()?,
                    &state,
                    match name {
                        "hybrid" => ExportMode::Hybrid,
                        "auto" => ExportMode::Auto,
                        "wavefront" => ExportMode::Wavefront,
                        _ => ExportMode::Tiled,
                    },
                ),
                None => ExportSession::resume(gpu_renderer()?, &state),
            };
            let mut session =
                resumed.with_context(|| format!("resuming from {}", state.display()))?;
            if let Some(milliseconds) = dispatch_ms {
                session.set_dispatch_budget_ms(milliseconds)?;
            }
            drive(
                session,
                &Drive {
                    out,
                    state_dir: Some(state),
                    checkpoint_every: checkpoint_every.or(Some(Duration::from_secs(60))),
                    preview_every,
                    time_budget,
                    pause_file,
                },
            )
        }
        Command::Info { state } => info(&state),
        Command::Grade {
            exr,
            out,
            scene,
            look,
            bits,
        } => grade(&exr, &out, scene.as_deref(), look.as_deref(), bits),
        Command::Graph { cmd } => match cmd {
            GraphCmd::Language { path, out } => {
                let graph = load_graph(&path)?;
                graph.validate_structure()?;
                std::fs::write(&out, syfrack_core::scene::graph::language::format(&graph))?;
                println!("wrote {}", out.display());
                Ok(())
            }
            GraphCmd::Build { path, out, scene } => {
                let mut scene = match scene {
                    Some(path) => Scene::load(&path)?,
                    None => Scene::example(),
                };
                let source = std::fs::read_to_string(&path)?;
                let graph = syfrack_core::scene::graph::language::compile(
                    &source,
                    Some(&scene.fractal.to_graph()),
                )?;
                graph.validate()?;
                scene.fractal = syfrack_core::Fractal::Graph(graph);
                scene.save(&out)?;
                println!("wrote {}", out.display());
                Ok(())
            }
            GraphCmd::Validate { path } => graph_validate(&path),
            GraphCmd::Compile { path, out } => graph_compile(&path, &out),
            GraphCmd::Upgrade { path, force } => graph_upgrade(&path, force),
            GraphCmd::Thumbnail { path, samples } => graph_thumbnail(&path, samples),
        },
    }
}

/// A scene's graph, or a bare graph file.
fn load_graph(path: &Path) -> Result<Graph> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    match Scene::from_ron(&text) {
        Ok(scene) => Ok(scene.fractal.to_graph()),
        Err(scene_err) => ron::from_str::<Graph>(&text).map_err(|graph_err| {
            anyhow::anyhow!(
                "{}: neither a scene ({scene_err}) nor a graph ({graph_err})",
                path.display()
            )
        }),
    }
}

fn graph_validate(path: &Path) -> Result<()> {
    let g = load_graph(path)?;
    g.validate()?;
    println!("nodes       : {}", g.nodes.len());
    println!("links       : {}", g.links.len());
    println!("active      : {}", g.active_nodes().len());
    println!("depth       : {}", g.iterate_depth());
    println!("iterations  : {}", g.iteration_product());
    println!("scalars     : {}", g.scalar_count());
    println!("lipschitz   : {:.3}", g.lipschitz(g.bounding_radius_hint));
    println!("step scale  : {:.3}", g.step_scale());
    println!("radius      : {:.3}", g.estimate_bounding_radius());
    Ok(())
}

fn graph_compile(path: &Path, out: &Path) -> Result<()> {
    let g = load_graph(path)?;
    g.validate()?;
    let compiled = syfrack_render::graph_wgsl::compile(&g)?;
    std::fs::write(out, compiled.wgsl).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn graph_upgrade(path: &Path, force: bool) -> Result<()> {
    // A pre-plan-4 scene fails here with « scène antérieure au plan 4 … ».
    let scene = Scene::load(path).with_context(|| format!("loading {}", path.display()))?;
    if !force {
        anyhow::bail!(
            "{} est déjà un graphe (already a graph scene); use --force to rewrite it",
            path.display()
        );
    }
    // Never overwrite a backup: .bak, then .bak.1, .bak.2, ...
    let mut backup = with_suffix(path, "bak");
    let mut n = 0;
    while backup.exists() {
        n += 1;
        backup = with_suffix(path, &format!("bak.{n}"));
    }
    std::fs::copy(path, &backup).with_context(|| format!("writing {}", backup.display()))?;
    scene.validate()?;
    scene
        .save(path)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("upgraded {} (backup {})", path.display(), backup.display());
    Ok(())
}

fn graph_thumbnail(path: &Path, samples: u32) -> Result<()> {
    if samples < 1 {
        anyhow::bail!("--samples must be at least 1, got {samples}");
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let scene = match Scene::from_ron(&text) {
        Ok(scene) => scene,
        Err(_) => {
            let g = load_graph(path)?;
            g.validate()?;
            thumbnail_scene(&g)
        }
    };
    // A scene file keeps its camera; a bare graph is framed by thumbnail_scene.
    let clay = clay_scene(&scene);
    clay.validate()?;
    let cfg = RenderConfig::new(320, 180, samples);
    let out = gpu_renderer()?.render(&clay, &cfg, &mut |_| {})?;
    let encoded = develop_and_encode(&out.color, &clay.look, 8);
    let png = path.with_extension("png");
    write_png(&png, &encoded, clay.look.output, None)
        .with_context(|| format!("writing {}", png.display()))?;
    println!("wrote {}", png.display());
    Ok(())
}

fn probe(cpu: bool) -> Result<()> {
    let ctx = if cpu {
        GpuContext::new_cpu()?
    } else {
        GpuContext::new()?
    };
    println!("adapter : {}", ctx.info.name);
    println!("backend : {:?}", ctx.info.backend);
    let out = ctx.run_probe()?;
    println!(
        "probe   : ok ({} values, last = {})",
        out.len(),
        out[out.len() - 1]
    );
    Ok(())
}

/// Strips a trailing .exr/.png/.ron so `--out render.exr` and `--out render` agree.
fn stem(out: &Path) -> PathBuf {
    match out.extension().and_then(|e| e.to_str()) {
        Some("exr") | Some("png") | Some("ron") => out.with_extension(""),
        _ => out.to_path_buf(),
    }
}

/// Appends `.ext` to the whole path, so dotted names like `run1.5` survive.
fn with_suffix(base: &Path, ext: &str) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn gpu_renderer() -> Result<Renderer> {
    let ctx = Arc::new(GpuContext::new()?);
    eprintln!("gpu: {} ({:?})", ctx.info.name, ctx.info.backend);
    Renderer::new(ctx)
}

fn info(dir: &Path) -> Result<()> {
    let path = dir.join("state.ron");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let st: SessionState =
        ron::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let min = st.tiles.iter().map(|t| t.done).min().unwrap_or(0);
    let max = st.tiles.iter().map(|t| t.done).max().unwrap_or(0);
    println!(
        "scene     : graph of {} nodes, seed {}",
        st.scene.fractal.graph().nodes.len(),
        st.scene.seed
    );
    println!(
        "image     : {}x{}, tile {}",
        st.cfg.width, st.cfg.height, st.tile_size
    );
    println!(
        "samples   : min {min}, max {max} / {}",
        st.cfg.samples_per_pixel
    );
    println!(
        "dispatches: {}, elapsed {:.1}s",
        st.dispatches, st.elapsed_secs
    );
    Ok(())
}

/// The twin `<out>.ron` must not overwrite `<state>/state.ron`.
fn check_out_vs_state(out: &Path, state_dir: &Path) -> Result<()> {
    let twin = with_suffix(&stem(out), "ron");
    let state_file = state_dir.join("state.ron");
    let norm = |p: &Path| -> PathBuf {
        let name = p.file_name().map(PathBuf::from).unwrap_or_default();
        let parent = p.parent().filter(|d| !d.as_os_str().is_empty());
        let parent = parent.unwrap_or(Path::new("."));
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(name)
    };
    if norm(&twin) == norm(&state_file) {
        anyhow::bail!(
            "--out {} would overwrite the state file {}; choose an --out that is not <state-dir>/state",
            out.display(),
            state_file.display()
        );
    }
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let out_dir = twin.parent().filter(|d| !d.as_os_str().is_empty());
    let out_dir = canon(out_dir.unwrap_or(Path::new(".")));
    if out_dir.starts_with(canon(state_dir)) {
        anyhow::bail!(
            "--out {} lies inside the state directory {}; choose an --out outside it",
            out.display(),
            state_dir.display()
        );
    }
    Ok(())
}

/// `render` starts from scratch: an existing non-empty state directory is
/// never overwritten (it may not even be a state directory).
fn check_state_dir_free(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    if !dir.is_dir() {
        anyhow::bail!("--state {} exists and is not a directory", dir.display());
    }
    let mut entries =
        std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    if entries.next().is_none() {
        return Ok(());
    }
    if dir.join("state.ron").exists() {
        anyhow::bail!(
            "{} already holds a render state; continue it with `syfrack resume {}` or choose another --state",
            dir.display(),
            dir.display()
        );
    }
    anyhow::bail!(
        "--state {} is not empty and is not a SyFrack state directory; refusing to use it",
        dir.display()
    );
}

fn write_preview(session: &ExportSession, base: &Path) -> Result<()> {
    let out = session.readback()?;
    let scene = session.scene();
    let filtered;
    let image = if session.config().denoise
        && session.scene().dimension == syfrack_core::Dimension::ThreeD
    {
        filtered = syfrack_render::denoise::session_output(session, &out);
        &filtered
    } else {
        &out.color
    };
    let encoded = develop_and_encode(image, &scene.look, 8);
    let path = with_suffix(base, "preview.png");
    write_png(&path, &encoded, scene.look.output, None)
        .with_context(|| format!("writing {}", path.display()))
}

struct Drive {
    out: PathBuf,
    state_dir: Option<PathBuf>,
    checkpoint_every: Option<Duration>,
    preview_every: Option<Duration>,
    time_budget: Option<Duration>,
    pause_file: Option<PathBuf>,
}

fn seconds_arg(name: &str, v: Option<f64>) -> Result<Option<Duration>> {
    match v {
        None => Ok(None),
        Some(t) if t.is_finite() && t >= 0.0 => Duration::try_from_secs_f64(t)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("--{name}: {e} (got {t})")),
        Some(t) => anyhow::bail!("--{name} must be a non-negative number of seconds, got {t}"),
    }
}

/// Steps the session to completion or until the budget expires, saving
/// checkpoints and previews on their intervals; then writes the outputs.
fn drive(mut session: ExportSession, d: &Drive) -> Result<()> {
    let start = Instant::now();
    let mut last_checkpoint = Instant::now();
    let mut last_preview = Instant::now();
    let mut last_report = 0u32;
    let mut last_print = Instant::now();
    let base = stem(&d.out);
    if let Some(dir) = base.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
    }
    let interrupted = loop {
        if d.pause_file.as_ref().is_some_and(|p| p.is_file()) {
            eprintln!("pause requested; saving render state");
            break true;
        }
        if let Some(budget) = d.time_budget {
            if start.elapsed() >= budget {
                break true;
            }
        }
        let done = session.step(Duration::from_millis(500))?;
        let p = session.progress();
        if p.dispatches - last_report >= 16
            || last_print.elapsed() >= Duration::from_secs(5)
            || done
        {
            last_print = Instant::now();
            last_report = p.dispatches;
            eprintln!(
                "  {:>5} dispatches  {:5.1}%  spp {:>5}..{:<5} / {}  tiles {}  max dispatch {} ms  {:.1}s",
                p.dispatches,
                p.fraction_complete() * 100.0,
                p.samples_min,
                p.samples_max,
                p.target,
                p.tiles,
                p.max_dispatch_ms,
                p.elapsed.as_secs_f32()
            );
        }
        if let (Some(dir), Some(every)) = (&d.state_dir, d.checkpoint_every) {
            if last_checkpoint.elapsed() >= every && !done {
                match session.save(dir) {
                    Ok(()) => eprintln!("  checkpoint saved to {}", dir.display()),
                    Err(e) => eprintln!("warning: checkpoint failed: {e:#}"),
                }
                last_checkpoint = Instant::now();
            }
        }
        if let Some(every) = d.preview_every {
            if last_preview.elapsed() >= every {
                if let Err(e) = write_preview(&session, &base) {
                    eprintln!("warning: preview failed: {e:#}");
                }
                last_preview = Instant::now();
            }
        }
        if done {
            break false;
        }
    };
    let output = session.readback()?;
    let scene = session.scene();
    eprintln!(
        "done: {}x{} spp {}..{} in {:.1}s",
        output.color.width,
        output.color.height,
        output.samples_min,
        output.samples_max,
        output.elapsed.as_secs_f32()
    );
    let ron = scene.to_ron();
    let exr_path = with_suffix(&base, "exr");
    write_exr(&exr_path, &output.color)
        .with_context(|| format!("writing {}", exr_path.display()))?;
    let filtered;
    let image = if session.config().denoise
        && session.scene().dimension == syfrack_core::Dimension::ThreeD
    {
        filtered = syfrack_render::denoise::session_output(&session, &output);
        &filtered
    } else {
        &output.color
    };
    let encoded = develop_and_encode(image, &scene.look, 16);
    let png_path = with_suffix(&base, "png");
    write_png(&png_path, &encoded, scene.look.output, Some(&ron))
        .with_context(|| format!("writing {}", png_path.display()))?;
    let ron_path = with_suffix(&base, "ron");
    std::fs::write(&ron_path, &ron).with_context(|| format!("writing {}", ron_path.display()))?;
    println!("wrote {}.{{exr,png,ron}}", base.display());
    // The outputs exist; a failing state save must not turn that into an error.
    if let Some(dir) = &d.state_dir {
        match session.save(dir) {
            Ok(()) => eprintln!(
                "{} state saved to {} (continue with `syfrack resume`)",
                if interrupted {
                    "render paused;"
                } else {
                    "render complete;"
                },
                dir.display()
            ),
            Err(e) => eprintln!(
                "warning: the outputs were written but saving the state to {} failed: {e:#}",
                dir.display()
            ),
        }
    }
    Ok(())
}

fn grade(
    exr: &Path,
    out: &Path,
    scene: Option<&Path>,
    look: Option<&Path>,
    bits: u8,
) -> Result<()> {
    if bits != 8 && bits != 16 {
        anyhow::bail!("--bits must be 8 or 16, got {bits}");
    }
    let hdr = read_exr(exr).with_context(|| format!("reading {}", exr.display()))?;
    let (look_value, ron) = match (scene, look) {
        (Some(s), _) => {
            let scene = Scene::load(s).with_context(|| format!("loading {}", s.display()))?;
            scene.validate()?;
            let ron = scene.to_ron();
            (scene.look, Some(ron))
        }
        (None, Some(l)) => {
            let look = Look::load(l).with_context(|| format!("loading {}", l.display()))?;
            (look, None)
        }
        (None, None) => anyhow::bail!("grade needs --scene or --look"),
    };
    let encoded = develop_and_encode(&hdr, &look_value, bits);
    write_png(out, &encoded, look_value.output, ron.as_deref())
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}
