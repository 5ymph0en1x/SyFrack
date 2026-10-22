//! Shared GPU preparation runtime and parameter captions for the one creation canvas.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use syfrack_render::Renderer;
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) renderer: Renderer,
    pub(crate) paused: Arc<AtomicBool>,
}
pub fn set_renderer(ctx: &egui::Context, renderer: &Renderer, paused: bool) {
    let id = egui::Id::new("natural-runtime");
    ctx.data_mut(|d| {
        let runtime = d.get_temp::<Runtime>(id).unwrap_or_else(|| Runtime {
            renderer: renderer.clone(),
            paused: Arc::new(AtomicBool::new(false)),
        });
        runtime.paused.store(paused, Ordering::Relaxed);
        d.insert_temp(id, runtime);
    });
}
pub(crate) fn shared_runtime(ctx: &egui::Context) -> Option<Runtime> {
    ctx.data(|d| d.get_temp::<Runtime>(egui::Id::new("natural-runtime")))
}
pub(crate) fn parameter_label(path: &str) -> String {
    let mut label = path
        .replace("simulations[", "Simulation ")
        .replace("guides[", "Guide ")
        .replace("seeds[", syfrack_i18n::tr!("Germe "))
        .replace("].", " · ");
    for (a, b) in [
        ("domain.", syfrack_i18n::tr!("Domaine · ")),
        ("bindings.", syfrack_i18n::tr!("Rendu · ")),
        ("film.", syfrack_i18n::tr!("Nacre · ")),
        ("lens.", syfrack_i18n::tr!("Lentille · ")),
        ("dispersion.", "Dispersion · "),
        ("center", "Centre"),
        ("half", syfrack_i18n::tr!("Demi-étendue")),
        ("resolution", syfrack_i18n::tr!("Résolution")),
        (
            "memory_budget_mib",
            syfrack_i18n::tr!("Budget mémoire (MiB)"),
        ),
        ("thickness_nm", syfrack_i18n::tr!("Épaisseur (nm)")),
        (
            "thickness_range_nm",
            syfrack_i18n::tr!("Variation d’épaisseur (nm)"),
        ),
        ("cauchy_b", "Cauchy B (µm²)"),
        ("max_steps", syfrack_i18n::tr!("Limite d’intégration")),
        ("escape_radius", syfrack_i18n::tr!("Rayon de sortie")),
        ("iterations", syfrack_i18n::tr!("Itérations")),
        ("steps", syfrack_i18n::tr!("Étapes")),
        ("dt", syfrack_i18n::tr!("Pas de temps")),
        ("strength", syfrack_i18n::tr!("Intensité")),
        ("radius", syfrack_i18n::tr!("Rayon")),
        ("phase", "Phase"),
        ("frequency", syfrack_i18n::tr!("Fréquence")),
        ("value", syfrack_i18n::tr!("Valeur")),
    ] {
        label = label.replace(a, syfrack_i18n::text(b));
    }
    label
}
