//! A colour study of the native fractal texture, sampled off the UI thread.
use std::sync::{mpsc, Arc, Mutex};

use egui::{ColorImage, TextureHandle, TextureOptions, Ui};
use glam::Vec3;
use syfrack_core::{FractalTexture, HdrImage, Look, Scene};

use super::theme;

type Request = (FractalTexture, Look, [f32; 3]);

#[derive(Default)]
struct Cache {
    request: Option<Request>,
    pending: Option<mpsc::Receiver<ColorImage>>,
    image: Option<TextureHandle>,
}

fn sample(request: &Request) -> ColorImage {
    let (texture, look, albedo) = request;
    let (w, h) = (80, 60);
    let mut image = HdrImage::new(w, h);
    let (width, depth) = texture
        .fan
        .as_ref()
        .map_or((4.0, 3.0), |f| (f.period[0] * 3.0, f.period[1] * 3.0));
    for y in 0..h {
        for x in 0..w {
            let p = Vec3::new(
                (x as f32 / w as f32 - 0.5) * width,
                0.0,
                y as f32 / h as f32 * depth,
            );
            let ramp = texture.ramp.sample(texture.value_at(p));
            let c = std::array::from_fn::<_, 3, _>(|i| {
                albedo[i] * (1.0 - texture.strength + texture.strength * ramp[i])
            });
            image.set(x, y, [c[0], c[1], c[2], 1.0]);
        }
    }
    let mut look = look.clone();
    look.vignette = None;
    look.grain = None;
    look.dither = false;
    // The study remains sRGB, like the Studio monitor.
    look.output = syfrack_core::OutputSpace::Srgb;
    let encoded = syfrack_color::develop_and_encode(&image, &look, 8);
    let rgb: Vec<u8> = encoded.data.iter().map(|&v| v as u8).collect();
    ColorImage::from_rgb([w as usize, h as usize], &rgb)
}

/// A visual-review runner can wait for the asynchronous native study.
pub fn is_pending(ctx: &egui::Context) -> bool {
    let cache =
        ctx.data(|d| d.get_temp::<Arc<Mutex<Cache>>>(egui::Id::new("fractal-texture-study")));
    cache.is_some_and(|c| c.lock().is_ok_and(|c| c.pending.is_some()))
}

pub fn show(ui: &mut Ui, scene: &Scene) {
    let Some(texture) = &scene.material.texture else {
        return;
    };
    let id = egui::Id::new("fractal-texture-study");
    let cache = ui.data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Cache>>>(id).clone());
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    if let Some(rx) = &cache.pending {
        match rx.try_recv() {
            Ok(image) => {
                if let Some(handle) = &mut cache.image {
                    handle.set(image, TextureOptions::LINEAR);
                } else {
                    cache.image = Some(ui.ctx().load_texture(
                        "native-fractal-study",
                        image,
                        TextureOptions::LINEAR,
                    ));
                }
                cache.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => cache.pending = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    let request = (texture.clone(), scene.look.clone(), scene.material.albedo);
    // One worker at a time; a slider release launches the latest request.
    if cache.pending.is_none()
        && cache.request.as_ref() != Some(&request)
        && !ui.input(|i| i.pointer.any_down())
    {
        cache.request = Some(request.clone());
        let (tx, rx) = mpsc::channel();
        cache.pending = Some(rx);
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let _ = tx.send(sample(&request));
            ctx.request_repaint();
        });
    }
    ui.horizontal(|ui| {
        if let Some(image) = &cache.image {
            ui.add(egui::Image::new((image.id(), egui::vec2(128.0, 96.0))));
        } else {
            ui.allocate_space(egui::vec2(128.0, 96.0));
        }
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(syfrack_i18n::tr!("Étude du motif")).strong());
            ui.label(
                egui::RichText::new(syfrack_i18n::tr!(
                    "Couleur · plan XZ\nRelief visible dans l’aperçu 3D"
                ))
                .small()
                .color(theme::TEXT_DIM),
            );
            if cache.pending.is_some() {
                ui.small(syfrack_i18n::tr!("Calcul du motif…"));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_study_is_finite_and_varied() {
        let image = sample(&(
            crate::ui::panels::material::preset(1),
            Look::neutral(),
            [0.9; 3],
        ));
        assert_eq!(image.size, [80, 60]);
        let min = image.pixels.iter().map(|p| p.r()).min().unwrap();
        let max = image.pixels.iter().map(|p| p.r()).max().unwrap();
        assert!(max - min > 20, "the native pattern is visible");
    }
}
