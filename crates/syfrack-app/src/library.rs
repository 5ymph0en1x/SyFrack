//! The form library: the `graphs/` folder as a list of named graphs, each
//! with an optional clay thumbnail (`<stem>.png`). The shipped forms take
//! their names from `BUNDLED`; a saved form has its name in `<stem>.name`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use syfrack_core::{thumbnail_scene, Graph, Params};
use syfrack_render::graph_wgsl::fingerprint;
use syfrack_render::{RenderConfig, Renderer};

/// Folder of the library, next to the executable or in the working directory.
pub const DIR: &str = "graphs";
/// The forms shipped with the studio: file stem and display name. Only
/// these are « bundled » (listed first); any other file is the user's.
pub const BUNDLED: &[(&str, &str)] = &[
    ("boite-bulbe", "Boîte bulbe"),
    ("cathedrale", "Cathédrale"),
    ("dentelle-inversee", "Dentelle inversée"),
    ("evidement", "Évidement"),
    ("mandelbox", "Mandelbox"),
    ("mandelbulb", "Mandelbulb"),
    ("menger", "Menger"),
    ("menger-kaleido", "Menger kaléido"),
    ("menger-torsade", "Menger torsadé"),
    ("rosace-plis", "Rosace plissée"),
    ("sphere", "Sphère"),
];

/// Size and samples of a library thumbnail (as `syfrack graph thumbnail`).
pub const THUMB_SIZE: (u32, u32) = (320, 180);
pub const THUMB_SAMPLES: u32 = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub name: String,
    pub nodes: usize,
    pub png: Option<PathBuf>,
    pub graph: Graph,
    /// Shipped with the studio (its stem is in `BUNDLED`): listed first.
    pub bundled: bool,
}

impl LibraryEntry {
    /// Localize shipped captions at draw time; preserve user-assigned names.
    pub fn localized_name(&self) -> &str {
        if self.bundled && BUNDLED.iter().any(|(_, name)| *name == self.name) {
            syfrack_i18n::text(&self.name)
        } else {
            &self.name
        }
    }
}

/// The library in the workspace or portable installation.
pub fn default_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(DIR)))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| crate::paths::content_root().join(DIR))
}

/// File stem to display name: `menger-kaleido` reads « Menger kaleido ».
fn display_name(stem: &str) -> String {
    let spaced = stem.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The `*.ron` files of `dir` that parse as a graph with a sound structure:
/// bundled ones first, then by name. Anything else is ignored.
pub fn scan(dir: &Path) -> Vec<LibraryEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<LibraryEntry> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let graph = ron::from_str::<Graph>(&text).ok()?;
            graph.validate_structure().ok()?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let shipped = BUNDLED.iter().find(|(s, _)| *s == stem).map(|(_, n)| *n);
            let named = std::fs::read_to_string(path.with_extension("name"))
                .ok()
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty());
            let png = Some(path.with_extension("png")).filter(|p| p.is_file());
            Some(LibraryEntry {
                name: named
                    .or_else(|| shipped.map(str::to_string))
                    .unwrap_or_else(|| display_name(&stem)),
                nodes: graph.nodes.len(),
                bundled: shipped.is_some(),
                png,
                graph,
                path,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.bundled
            .cmp(&a.bundled)
            // Accents do not move a name to the end.
            .then_with(|| slug(&a.name).cmp(&slug(&b.name)))
    });
    out
}

/// Every scalar of `g`, in visiting order.
fn values(g: &Graph) -> Vec<f32> {
    let mut g = g.clone();
    let mut out = Vec::new();
    g.visit_params("", &mut |_, v, _| out.push(*v));
    out
}

/// The entry `g` is a copy of: same topology and same values (names and
/// positions on the canvas do not count).
pub fn matching<'a>(entries: &'a [LibraryEntry], g: &Graph) -> Option<&'a LibraryEntry> {
    let fp = fingerprint(g);
    let vals = values(g);
    entries
        .iter()
        .find(|e| fingerprint(&e.graph) == fp && values(&e.graph) == vals)
}

/// Lowercase ASCII file stem: accents stripped, every run of other
/// characters one `-`. Never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars().flat_map(char::to_lowercase) {
        let base: Option<&str> = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => Some("a"),
            'æ' => Some("ae"),
            'ç' => Some("c"),
            'è' | 'é' | 'ê' | 'ë' => Some("e"),
            'ì' | 'í' | 'î' | 'ï' => Some("i"),
            'ñ' => Some("n"),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => Some("o"),
            'œ' => Some("oe"),
            'ù' | 'ú' | 'û' | 'ü' => Some("u"),
            'ý' | 'ÿ' => Some("y"),
            'ß' => Some("ss"),
            _ => None,
        };
        let mut push = |t: &str| {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push_str(t);
        };
        match base {
            Some(t) => push(t),
            None if c.is_ascii_alphanumeric() => push(c.encode_utf8(&mut [0; 4])),
            None => dash = true,
        }
    }
    if out.is_empty() {
        "forme".to_string()
    } else {
        out
    }
}

/// Refuses a name whose file would be a shipped form: those are embedded
/// at build time (`Scene::example`, `--kind`, the parity baselines), so
/// they are never written from the studio. Suggests « <name>-2 ».
pub fn refuse_bundled(name: &str) -> Result<()> {
    let stem = slug(name);
    if BUNDLED.iter().any(|(s, _)| *s == stem) {
        return Err(anyhow!(syfrack_i18n::tr_format!(
            "forme livrée, choisissez un autre nom (par exemple « {}-2 »)",
            name.trim()
        )));
    }
    Ok(())
}

/// Writes `g` as `dir/<slug>.ron` with its display name in `<slug>.name`.
/// Refuses an existing file: overwriting is the panel's decision. Refuses
/// a shipped form's name (`refuse_bundled`).
pub fn save(dir: &Path, name: &str, g: &Graph) -> Result<PathBuf> {
    refuse_bundled(name)?;
    let path = dir.join(format!("{}.ron", slug(name)));
    if path.exists() {
        return Err(anyhow!(syfrack_i18n::tr_format!(
            "{} existe déjà",
            path.display()
        )));
    }
    write(dir, name, g, &path)
}

/// `save` over an existing file, once the user confirmed. The old
/// thumbnail goes: it shows the old form (a new one is rendered when the
/// form is complete). A shipped form is never replaced (`refuse_bundled`).
pub fn overwrite(dir: &Path, name: &str, g: &Graph) -> Result<PathBuf> {
    refuse_bundled(name)?;
    let path = dir.join(format!("{}.ron", slug(name)));
    let png = path.with_extension("png");
    if png.exists() {
        std::fs::remove_file(&png)
            .with_context(|| syfrack_i18n::tr_format!("suppression de {}", png.display()))?;
    }
    write(dir, name, g, &path)
}

fn write(dir: &Path, name: &str, g: &Graph, path: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| syfrack_i18n::tr_format!("création de {}", dir.display()))?;
    let text = ron::ser::to_string_pretty(g, ron::ser::PrettyConfig::new())?;
    std::fs::write(path, text)
        .with_context(|| syfrack_i18n::tr_format!("écriture de {}", path.display()))?;
    std::fs::write(path.with_extension("name"), name.trim()).with_context(|| {
        syfrack_i18n::tr_format!("écriture de {}", path.with_extension("name").display())
    })?;
    Ok(path.to_path_buf())
}

/// Renders the clay thumbnail of `g` to `png` (blocking: call it on a thread).
pub fn render_thumbnail(renderer: &Renderer, g: &Graph, png: &Path) -> Result<()> {
    let clay = thumbnail_scene(g);
    clay.validate()?;
    let cfg = RenderConfig::new(THUMB_SIZE.0, THUMB_SIZE.1, THUMB_SAMPLES);
    let out = renderer.render(&clay, &cfg, &mut |_| {})?;
    let encoded = syfrack_color::develop_and_encode(&out.color, &clay.look, 8);
    syfrack_color::write_png(png, &encoded, clay.look.output, None)
        .with_context(|| syfrack_i18n::tr_format!("écriture de {}", png.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slug_strips_accents_and_spaces() {
        assert_eq!(slug("Boîte à bulbe  № 2"), "boite-a-bulbe-2");
    }
    #[test]
    fn scan_lists_graph_files_and_ignores_the_rest() {
        let dir = std::env::temp_dir().join(format!("syfrack-lib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save(&dir, "Ma forme", &syfrack_core::Graph::new_minimal()).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("broken.ron"), "(nodes: [").unwrap();
        let entries = scan(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Ma forme");
        assert_eq!(entries[0].nodes, 3);
        assert!(
            save(&dir, "Ma forme", &syfrack_core::Graph::new_minimal()).is_err(),
            "no silent overwrite"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bundled_graphs_come_first_with_their_names_and_user_files_follow() {
        let dir = std::env::temp_dir().join(format!("syfrack-lib-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = syfrack_core::Graph::new_minimal();
        let text = ron::ser::to_string_pretty(&g, Default::default()).unwrap();
        std::fs::write(dir.join("menger-kaleido.ron"), &text).unwrap();
        // A user file without a .name: never bundled, named from its stem.
        std::fs::write(dir.join("zz-forme.ron"), &text).unwrap();
        save(&dir, "Aaa", &g).unwrap();
        let entries = scan(&dir);
        let seen: Vec<(&str, bool)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.bundled))
            .collect();
        assert_eq!(
            seen,
            [
                ("Menger kaléido", true),
                ("Aaa", false),
                ("Zz forme", false)
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwriting_a_form_drops_its_old_thumbnail() {
        let dir = std::env::temp_dir().join(format!("syfrack-lib-over-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = syfrack_core::Graph::new_minimal();
        let ron = save(&dir, "Forme", &g).unwrap();
        let png = ron.with_extension("png");
        std::fs::write(&png, b"old").unwrap();
        overwrite(&dir, "Forme", &g).unwrap();
        assert!(ron.is_file());
        assert!(!png.exists(), "a stale thumbnail would show the old form");
        assert_eq!(scan(&dir)[0].png, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_shipped_form_is_never_replaced() {
        let dir = std::env::temp_dir().join(format!("syfrack-lib-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shipped = syfrack_core::Graph::bundled("sphere").unwrap();
        let text = ron::ser::to_string_pretty(&shipped, Default::default()).unwrap();
        let ron = dir.join("sphere.ron");
        std::fs::write(&ron, &text).unwrap();
        std::fs::write(ron.with_extension("png"), b"thumb").unwrap();
        let g = syfrack_core::Graph::new_minimal();
        let err = overwrite(&dir, "Sphère", &g).unwrap_err().to_string();
        assert!(err.contains("Bundled form; choose another name"), "{err}");
        assert!(err.contains("“Sphère-2”"), "{err}");
        assert_eq!(std::fs::read_to_string(&ron).unwrap(), text, "untouched");
        assert!(ron.with_extension("png").exists(), "thumbnail kept");
        assert!(
            save(&dir, " Mandelbox ", &g).is_err(),
            "no new shipped stem"
        );
        assert!(!dir.join("mandelbox.ron").exists());
        // The suggested name is free.
        save(&dir, "Sphère-2", &g).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_matches_its_entry_until_a_value_changes() {
        let g = syfrack_core::Graph::bundled("mandelbox").unwrap();
        let entry = LibraryEntry {
            path: PathBuf::from("mandelbox.ron"),
            name: "Mandelbox".into(),
            nodes: g.nodes.len(),
            png: None,
            graph: g.clone(),
            bundled: true,
        };
        let entries = [entry];
        let mut moved = g.clone();
        moved.nodes[0].pos = [100.0, 50.0];
        moved.nodes[0].name = "ailleurs".into();
        assert_eq!(
            matching(&entries, &moved).map(|e| e.name.as_str()),
            Some("Mandelbox")
        );
        let mut changed = g.clone();
        changed.safety = 0.5;
        assert!(matching(&entries, &changed).is_none());
    }

    #[test]
    fn the_bundled_library_scans_whole() {
        // A copy of the shipped forms (embedded RON and the shipped
        // thumbnails), never the live graphs/ folder that holds user saves.
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs");
        let dir = std::env::temp_dir().join(format!("syfrack-lib-whole-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (stem, text) in syfrack_core::scene::graph::bundled::BUNDLED {
            std::fs::write(dir.join(format!("{stem}.ron")), text).unwrap();
            let png = format!("{stem}.png");
            std::fs::copy(shipped.join(&png), dir.join(&png)).unwrap();
        }
        let entries = scan(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(entries.len(), BUNDLED.len(), "{entries:?}");
        // the studio names exactly the graphs the core embeds
        let mut core: Vec<&str> = syfrack_core::scene::graph::bundled::BUNDLED
            .iter()
            .map(|(s, _)| *s)
            .collect();
        core.sort_unstable();
        let stems: Vec<&str> = BUNDLED.iter().map(|(s, _)| *s).collect();
        assert_eq!(stems, core);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        let expected: Vec<&str> = BUNDLED.iter().map(|(_, n)| *n).collect();
        assert_eq!(names, expected, "bundled, by name");
        assert!(entries.iter().all(|e| e.png.is_some() && e.bundled));
        assert!(entries.iter().any(|e| e.name == "Évidement"));
    }
}
