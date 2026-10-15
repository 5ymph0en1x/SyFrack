//! Make small documentation previews from existing native renders. No rendering or AI generation.
//! cargo run -p syfrack-app --example release_images -- source.png destination.png [...]
use anyhow::{bail, Result};
use std::{fs::File, io::BufWriter, path::Path};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() % 2 != 0 {
        bail!("expected source.png destination.png pairs");
    }
    for pair in args.chunks_exact(2) {
        let (width, height, rgba) = syfrack_app_lib::gallery::thumbnail(Path::new(&pair[0]), 1600)?;
        let path = Path::new(&pair[1]);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgba)?;
        println!(
            "{}: {} x {} documentation preview",
            path.display(),
            width,
            height
        );
    }
    Ok(())
}
