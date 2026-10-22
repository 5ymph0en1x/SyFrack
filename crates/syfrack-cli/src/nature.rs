use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use std::{path::PathBuf, sync::Arc};
use syfrack_core::{scene::phenomena::*, Scene};

#[derive(Subcommand)]
pub enum NatureCommand {
    /// List editable natural-field experiments.
    List,
    /// Write an editable built-in experiment.
    Preset {
        name: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Inspect field types, bindings and bounded snapshot settings.
    Inspect { scene: PathBuf },
    /// Resolve animation tracks into an immutable scene at a chosen time.
    Frame {
        scene: PathBuf,
        #[arg(long)]
        time: f32,
        #[arg(long)]
        out: PathBuf,
    },
    /// Freeze exact simulated states inside a portable scene file.
    Freeze {
        scene: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Create selected-parameter variations with persistent parent/child lineage.
    Variants {
        scene: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        parameters: Vec<String>,
        #[arg(long,default_value_t=12,value_parser=clap::value_parser!(u32).range(1..=64))]
        count: u32,
        #[arg(long, default_value_t = 0.1)]
        strength: f32,
        #[arg(long, default_value_t = 7)]
        seed: u64,
    },
}
fn save(scene: &Scene, out: &PathBuf) -> Result<()> {
    ensure!(!out.exists(), "output already exists: {}", out.display());
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    scene.render_scene()?.save(out)?;
    println!("{}", out.display());
    Ok(())
}
pub fn run(command: NatureCommand) -> Result<()> {
    match command {
        NatureCommand::List => {
            for (key, label) in presets::PRESETS {
                println!("{key:16} {label}");
            }
        }
        NatureCommand::Preset { name, out } => save(&presets::scene(&name)?, &out)?,
        NatureCommand::Inspect { scene } => {
            let s = Scene::load(scene)?;
            let p = s.phenomena.context("scene has no natural fields")?;
            p.validate()?;
            let types = p.fields.validate(&p.simulations)?;
            for id in p.fields.order()? {
                let n = p.fields.nodes.iter().find(|n| n.id == id).unwrap();
                println!("{} {:?} {} <- {:?}", id, types[&id], n.name, n.inputs);
            }
            println!(
                "domain: {:?}; {} cells; {} MiB budget",
                p.domain.resolution,
                p.domain.cells(),
                p.domain.memory_budget_mib
            );
            for (i, s) in p.simulations.iter().enumerate() {
                println!(
                    "snapshot {i}: {}; {} steps at dt={}; frozen={}",
                    s.name,
                    s.steps,
                    s.dt,
                    s.frozen.is_some()
                );
            }
        }
        NatureCommand::Frame { scene, time, out } => {
            save(&Phenomena::scene_at(&Scene::load(scene)?, time)?, &out)?
        }
        NatureCommand::Freeze { scene, out } => {
            let s = Scene::load(scene)?;
            let r = syfrack_render::Renderer::new(Arc::new(syfrack_render::GpuContext::new()?))?;
            let prepared = r
                .prepare_phenomena(&s)?
                .context("scene has no field snapshots")?;
            let s = r.freeze_creation_sources(&s)?;
            for (i, d) in prepared.diagnostics.iter().enumerate() {
                println!(
                    "snapshot {i}: mass {} -> {}, eigen residual {}, occupied {}",
                    d.mass_before, d.mass_after, d.residual, d.occupied
                );
            }
            save(&s, &out)?;
        }
        NatureCommand::Variants {
            scene,
            out,
            parameters,
            count,
            strength,
            seed,
        } => {
            ensure!(!out.exists(), "variant destination already exists");
            let s = Scene::load(scene)?;
            let variants =
                VariantAtlas::variations(&s, &parameters, count as usize, strength, seed)?;
            let mut atlas = VariantAtlas::default();
            let parent = atlas.insert(None, "Source".into(), s)?;
            std::fs::create_dir_all(&out)?;
            for (i, v) in variants.into_iter().enumerate() {
                let name = format!("variant-{:02}", i + 1);
                v.save(out.join(format!("{name}.ron")))?;
                atlas.insert(Some(parent), name, v)?;
            }
            std::fs::write(
                out.join("atlas.ron"),
                ron::ser::to_string_pretty(&atlas, Default::default())?,
            )?;
            println!("{} saved branches in {}", count, out.display());
        }
    }
    Ok(())
}
