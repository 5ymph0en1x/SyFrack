//! The forms shipped in `graphs/`, embedded in the binary: the classic
//! fractals (now only graphs), the sphere on its ground of `Scene::example`,
//! and the library compositions.

use super::Graph;

/// File stem and RON text of every bundled graph.
pub const BUNDLED: &[(&str, &str)] = &[
    ("sphere", include_str!("../../../../../graphs/sphere.ron")),
    (
        "mandelbox",
        include_str!("../../../../../graphs/mandelbox.ron"),
    ),
    ("menger", include_str!("../../../../../graphs/menger.ron")),
    (
        "rosace-plis",
        include_str!("../../../../../graphs/rosace-plis.ron"),
    ),
    (
        "dentelle-inversee",
        include_str!("../../../../../graphs/dentelle-inversee.ron"),
    ),
    (
        "menger-torsade",
        include_str!("../../../../../graphs/menger-torsade.ron"),
    ),
    (
        "mandelbulb",
        include_str!("../../../../../graphs/mandelbulb.ron"),
    ),
    (
        "boite-bulbe",
        include_str!("../../../../../graphs/boite-bulbe.ron"),
    ),
    (
        "menger-kaleido",
        include_str!("../../../../../graphs/menger-kaleido.ron"),
    ),
    (
        "evidement",
        include_str!("../../../../../graphs/evidement.ron"),
    ),
    (
        "cathedrale",
        include_str!("../../../../../graphs/cathedrale.ron"),
    ),
];

/// The bundled graph of that file stem, `None` for an unknown name.
pub fn bundled(name: &str) -> Option<Graph> {
    BUNDLED
        .iter()
        .find(|(stem, _)| *stem == name)
        .map(|(_, text)| ron::from_str(text).expect("bundled graphs parse"))
}

impl Graph {
    /// The bundled graph of that file stem (see `BUNDLED`).
    pub fn bundled(name: &str) -> Option<Graph> {
        bundled(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_graph_parses_and_validates() {
        assert_eq!(BUNDLED.len(), 11);
        for (name, _) in BUNDLED {
            let g = bundled(name).unwrap();
            g.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        assert!(bundled("cube").is_none());
    }

    #[test]
    fn the_shipped_files_on_disk_equal_the_embedded_graphs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs");
        for (stem, text) in BUNDLED {
            let path = dir.join(format!("{stem}.ron"));
            let disk = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            assert_eq!(disk, *text, "{stem}");
        }
    }
}
