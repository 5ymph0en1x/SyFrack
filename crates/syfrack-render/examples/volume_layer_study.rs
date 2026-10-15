//! Export native radiance layers and reconstruction ablations from a checkpoint.
use anyhow::Result;
use std::{path::Path, sync::Arc};
use syfrack_core::HdrImage;
use syfrack_render::{GpuContext, RenderSession, Renderer};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 2, "state directory, output directory");
    let out = Path::new(&args[1]);
    std::fs::create_dir_all(out)?;
    let session = RenderSession::resume(
        Renderer::new(Arc::new(GpuContext::new()?))?,
        Path::new(&args[0]),
    )?;
    let output = session.readback()?;
    let layers = output.layers.as_ref().expect("fog layers");
    let save = |name: &str, image: &HdrImage| -> Result<()> {
        syfrack_color::write_exr(&out.join(format!("{name}.exr")), image)?;
        syfrack_color::write_png(
            &out.join(format!("{name}.png")),
            &syfrack_color::develop_and_encode(image, &session.scene().look, 16),
            session.scene().look.output,
            None,
        )?;
        Ok(())
    };
    save("raw", &output.color)?;
    save("raw-volume", &layers.volume)?;
    save("raw-surface", &layers.surface)?;
    let (albedo, normals) = session.export_surface_guides()?;
    let t: Vec<f32> = normals.data.iter().map(|p| p[3]).collect();
    std::fs::write(out.join("transmission.bin"), bytemuck::cast_slice(&t))?;
    let components = layers
        .components
        .as_ref()
        .expect("direct/indirect components fit device limits");
    save("direct-volume", &components.direct)?;
    let indirect = HdrImage::from_data(
        output.color.width,
        output.color.height,
        layers
            .volume
            .data
            .iter()
            .zip(&components.direct.data)
            .map(|(v, d)| [v[0] - d[0], v[1] - d[1], v[2] - d[2], 1.0])
            .collect(),
    );
    save("indirect-volume", &indirect)?;
    save(
        "continuous-direct",
        &syfrack_render::volume_reconstruction::reconstruct(
            &components.direct,
            &components.direct_variance,
            &t,
        ),
    )?;
    save(
        "continuous-indirect",
        &syfrack_render::volume_reconstruction::reconstruct_indirect(
            &indirect,
            &components.indirect_variance,
            &t,
        ),
    )?;
    let volume = syfrack_render::volume_reconstruction::reconstruct(
        &layers.volume,
        &layers.volume_variance,
        &t,
    );
    save("continuous-volume", &volume)?;
    save(
        "separate-reconstruction",
        &syfrack_render::denoise::session_output(&session, &output),
    )?;
    if let Some(runtime) = syfrack_render::oidn::runtime() {
        let (mixed_a, mixed_n) = session.export_guides()?;
        save(
            "mixed-guides",
            &syfrack_render::oidn::filter_guided(
                &output.color,
                Some((&mixed_a, &mixed_n)),
                &runtime,
            )?,
        )?;
        save(
            "surface-reconstruction",
            &syfrack_render::oidn::filter_guided(
                &layers.surface,
                Some((&albedo, &normals)),
                &runtime,
            )?,
        )?;
    }
    let max_error = output
        .color
        .data
        .iter()
        .zip(&layers.surface.data)
        .zip(&layers.volume.data)
        .flat_map(|((r, s), v)| (0..3).map(move |c| (r[c] - s[c] - v[c]).abs()))
        .fold(0.0f32, f32::max);
    println!("Raw layer recomposition max absolute error: {max_error}");
    println!(
        "{}x{} {}..{} spp; {}",
        output.color.width,
        output.color.height,
        output.samples_min,
        output.samples_max,
        out.display()
    );
    Ok(())
}
