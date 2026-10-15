//! The renders folder as a gallery: one entry per output stem (PNG, scene RON,
//! EXR, `.state` directory), thumbnails decoded on a worker thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use syfrack_color::{read_png_text, SCENE_KEYWORD};
use syfrack_core::Scene;
use syfrack_render::SessionState;

/// Width of the thumbnails the worker produces.
const THUMB_W: u32 = 320;

pub struct StateInfo {
    pub samples_min: u32,
    pub samples_max: u32,
    pub target: u32,
    pub elapsed_secs: f64,
    pub dir: PathBuf,
}

impl StateInfo {
    /// Some tile is still short of its target: « Reprendre » has work to do.
    pub fn resumable(&self) -> bool {
        self.samples_min < self.target
    }
}

pub struct Entry {
    /// `<dir>/<name>` without extension.
    pub stem: PathBuf,
    pub name: String,
    pub png: Option<PathBuf>,
    pub ron: Option<PathBuf>,
    pub exr: Option<PathBuf>,
    /// From `<stem>.state/state.ron` when present and readable.
    pub state: Option<StateInfo>,
    /// From the PNG header.
    pub size: Option<(u32, u32)>,
    pub modified: Option<SystemTime>,
}

fn png_size(path: &Path) -> Option<(u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

fn read_state(dir: &Path) -> Option<StateInfo> {
    let text = std::fs::read_to_string(dir.join("state.ron")).ok()?;
    let st: SessionState = ron::from_str(&text).ok()?;
    let min = st.tiles.iter().map(|t| t.done).min().unwrap_or(0);
    let max = st.tiles.iter().map(|t| t.done).max().unwrap_or(0);
    Some(StateInfo {
        samples_min: min,
        samples_max: max,
        target: st.cfg.samples_per_pixel,
        elapsed_secs: st.elapsed_secs,
        dir: dir.to_path_buf(),
    })
}

/// Lists the renders of a directory, one entry per stem, newest first.
/// `*.preview.png`, directories other than `*.state` and anything in
/// `candidates/` are ignored.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut map: HashMap<String, Entry> = HashMap::new();
    fn entry_for<'a>(map: &'a mut HashMap<String, Entry>, dir: &Path, name: &str) -> &'a mut Entry {
        map.entry(name.to_string()).or_insert_with(|| Entry {
            stem: dir.join(name),
            name: name.to_string(),
            png: None,
            ron: None,
            exr: None,
            state: None,
            size: None,
            modified: None,
        })
    }
    for item in read.flatten() {
        let path = item.path();
        let file_name = item.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if let Some(stem) = file_name.strip_suffix(".state") {
                if let Some(info) = read_state(&path) {
                    let modified = mtime(&path.join("state.ron"));
                    let e = entry_for(&mut map, dir, stem);
                    e.state = Some(info);
                    e.modified = e.modified.max(modified);
                }
            }
            continue;
        }
        if file_name.ends_with(".preview.png") {
            continue;
        }
        let Some((stem, ext)) = file_name.rsplit_once('.') else {
            continue;
        };
        let modified = mtime(&path);
        match ext {
            "png" => {
                let e = entry_for(&mut map, dir, stem);
                e.size = png_size(&path);
                e.png = Some(path);
                e.modified = e.modified.max(modified);
            }
            "ron" => {
                let e = entry_for(&mut map, dir, stem);
                e.ron = Some(path);
                e.modified = e.modified.max(modified);
            }
            "exr" => {
                let e = entry_for(&mut map, dir, stem);
                e.exr = Some(path);
                e.modified = e.modified.max(modified);
            }
            _ => {}
        }
    }
    // A lone scene file is a scene, not a render.
    let mut entries: Vec<Entry> = map
        .into_values()
        .filter(|e| e.png.is_some() || e.exr.is_some() || e.state.is_some())
        .collect();
    entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

/// The scene embedded in the PNG, else the twin RON file.
pub fn scene_from_entry(e: &Entry) -> Result<Scene> {
    scene_with_source(e).map(|(scene, _)| scene)
}

/// Like `scene_from_entry`, also telling whether the scene came from the twin
/// RON (true) or from the PNG (false). A corrupt PNG scene falls through to the
/// RON; when both fail, both errors are reported.
pub fn scene_with_source(e: &Entry) -> Result<(Scene, bool)> {
    let mut png_err = None;
    if let Some(png) = &e.png {
        match read_png_text(png, SCENE_KEYWORD) {
            Ok(Some(text)) => match Scene::from_ron(&text) {
                Ok(scene) => return Ok((scene, false)),
                Err(err) => png_err = Some(format!("scene embedded in the png: {err}")),
            },
            Ok(None) => {}
            Err(err) => png_err = Some(format!("png: {err}")),
        }
    }
    let mut ron_err = None;
    if let Some(ron) = &e.ron {
        match Scene::load(ron) {
            Ok(scene) => return Ok((scene, true)),
            Err(err) => ron_err = Some(format!("twin ron: {err}")),
        }
    }
    let reasons: Vec<String> = png_err.into_iter().chain(ron_err).collect();
    if reasons.is_empty() {
        Err(anyhow!("{}: no scene in the PNG and no twin .ron", e.name))
    } else {
        Err(anyhow!("{}: {}", e.name, reasons.join("; ")))
    }
}

/// Removes the files of one entry: PNG, RON, EXR and its `.state` directory.
/// Anything else in the folder stays.
pub fn delete_entry(e: &Entry) -> Result<()> {
    for file in [&e.png, &e.ron, &e.exr].into_iter().flatten() {
        std::fs::remove_file(file).with_context(|| format!("delete {}", file.display()))?;
    }
    if let Some(st) = &e.state {
        std::fs::remove_dir_all(&st.dir).with_context(|| format!("delete {}", st.dir.display()))?;
    }
    Ok(())
}

/// RGBA8 thumbnail by box downscale. Accepts any PNG depth (16-bit keeps its
/// high byte). The longest side is bounded, including portrait images.
pub fn thumbnail(png: &Path, max_side: u32) -> Result<(u32, u32, Vec<u8>)> {
    thumbnail_with_stop(png, max_side, None)
}
fn thumbnail_with_stop(
    png: &Path,
    max_side: u32,
    stop: Option<&AtomicBool>,
) -> Result<(u32, u32, Vec<u8>)> {
    let file = std::fs::File::open(png).with_context(|| format!("open {}", png.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    // Always 8 bits per sample: palettes and 1/2/4-bit greys are expanded.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().context("png header")?;
    let mut buf = vec![0; reader.output_buffer_size().context("png size")?];
    let info = reader.next_frame(&mut buf).context("png frame")?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Indexed => return Err(anyhow!("indexed png not supported")),
    };
    if info.bit_depth != png::BitDepth::Eight || buf.len() < w * h * channels {
        return Err(anyhow!("unexpected png layout"));
    }
    let linear: [f32; 256] =
        std::array::from_fn(|i| syfrack_color::encode::srgb_to_linear(i as f32 / 255.0));
    let longest = w.max(h) as u64;
    let side = u64::from(max_side.max(1)).min(longest);
    // Integer aspect fitting also avoids losing a pixel to a rounded scale.
    let tw = ((w as u64 * side / longest) as usize).max(1);
    let th = ((h as u64 * side / longest) as usize).max(1);
    let mut out = Vec::with_capacity(tw * th * 4);
    for ty in 0..th {
        if stop.is_some_and(|s| s.load(Ordering::Relaxed)) {
            return Err(anyhow!("thumbnail cancelled"));
        }
        for tx in 0..tw {
            let x0 = tx as f32 * w as f32 / tw as f32;
            let y0 = ty as f32 * h as f32 / th as f32;
            let x1 = (tx + 1) as f32 * w as f32 / tw as f32;
            let y1 = (ty + 1) as f32 * h as f32 / th as f32;
            let area = (x1 - x0) * (y1 - y0);
            let mut rgb = [0.0; 3];
            let mut coverage = 0.0;
            for y in y0.floor() as usize..(y1.ceil() as usize).min(h) {
                let wy = (y1.min((y + 1) as f32) - y0.max(y as f32)).max(0.0);
                for x in x0.floor() as usize..(x1.ceil() as usize).min(w) {
                    let weight = wy * (x1.min((x + 1) as f32) - x0.max(x as f32)).max(0.0);
                    let index = (y * w + x) * channels;
                    let alpha = match channels {
                        2 => buf[index + 1],
                        4 => buf[index + 3],
                        _ => 255,
                    } as f32
                        / 255.0;
                    let covered = alpha * weight;
                    coverage += covered;
                    for (c, sum) in rgb.iter_mut().enumerate() {
                        let channel = if channels <= 2 { 0 } else { c };
                        *sum += linear[buf[index + channel] as usize] * covered;
                    }
                }
            }
            let alpha = (coverage / area * 255.0).round().clamp(0.0, 255.0) as u8;
            for c in rgb {
                let value = if alpha == 0 {
                    0.0
                } else {
                    syfrack_color::encode::linear_to_srgb(c / coverage)
                };
                out.push((value * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            out.push(alpha);
        }
    }
    Ok((tw as u32, th as u32, out))
}

/// `YYYY-MM-DD HH:MM` in UTC from a Unix time in seconds.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from a day count (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

type Thumb = (u32, u32, Vec<u8>);
type Decoded = (PathBuf, Option<SystemTime>, Result<Thumb>);

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Decodes thumbnails on one worker thread; results are polled each frame.
/// The cache is keyed by path and checked against the file's mtime.
pub struct Thumbnails {
    cache: HashMap<PathBuf, (Option<SystemTime>, Thumb)>,
    pending: HashSet<PathBuf>,
    /// Files that failed to decode, with the mtime they failed at.
    failed: HashMap<PathBuf, Option<SystemTime>>,
    jobs: Option<Sender<(PathBuf, Option<SystemTime>)>>,
    results: Receiver<Decoded>,
    worker: Option<JoinHandle<()>>,
    /// Set on drop: the worker discards its pending jobs.
    stop: Arc<AtomicBool>,
}

impl Default for Thumbnails {
    fn default() -> Self {
        Self::new()
    }
}

impl Thumbnails {
    pub fn new() -> Self {
        let (jobs, job_rx) = mpsc::channel::<(PathBuf, Option<SystemTime>)>();
        let (res_tx, results) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = std::thread::Builder::new()
            .name("syfrack-thumbnails".into())
            .spawn(move || {
                // Ends when the job sender is dropped or `stop` is raised: the
                // queued jobs are then dropped without being decoded.
                for (path, stamp) in job_rx {
                    if worker_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let r = std::panic::catch_unwind(|| {
                        thumbnail_with_stop(&path, THUMB_W, Some(&worker_stop))
                    })
                    .unwrap_or_else(|_| Err(anyhow!("thumbnail decoder panicked")));
                    if res_tx.send((path, stamp, r)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn thumbnail thread");
        Self {
            cache: HashMap::new(),
            pending: HashSet::new(),
            failed: HashMap::new(),
            jobs: Some(jobs),
            results,
            worker: Some(worker),
            stop,
        }
    }

    /// Queues a decode unless the cached one matches the file's mtime.
    pub fn request(&mut self, png: &Path) {
        let stamp = mtime(png);
        if self.pending.contains(png)
            || self.cache.get(png).is_some_and(|(s, _)| *s == stamp)
            || self.failed.get(png) == Some(&stamp)
        {
            return;
        }
        if let Some(jobs) = &self.jobs {
            if jobs.send((png.to_path_buf(), stamp)).is_ok() {
                self.pending.insert(png.to_path_buf());
            }
        }
    }

    pub fn poll(&mut self) {
        for (path, stamp, r) in self.results.try_iter() {
            self.pending.remove(&path);
            match r {
                Ok(t) => {
                    self.failed.remove(&path);
                    self.cache.insert(path, (stamp, t));
                }
                // Remembered, so the next scan does not queue it again.
                Err(_) => {
                    self.failed.insert(path, stamp);
                }
            }
        }
    }

    pub fn get(&self, png: &Path) -> Option<&Thumb> {
        self.cache.get(png).map(|(_, t)| t)
    }

    /// The mtime the cached thumbnail was decoded at: it changes when the
    /// thumbnail is refreshed.
    pub fn stamp(&self, png: &Path) -> Option<Option<SystemTime>> {
        self.cache.get(png).map(|(s, _)| *s)
    }

    /// The last decode of this file failed and the file has not changed since.
    pub fn has_failed(&self, png: &Path) -> bool {
        self.failed.get(png) == Some(&mtime(png))
    }

    pub fn is_pending(&self, png: &Path) -> bool {
        self.pending.contains(png)
    }

    /// Drops the thumbnails of files that left the folder.
    pub fn forget_except(&mut self, keep: &HashSet<PathBuf>) {
        self.cache.retain(|p, _| keep.contains(p));
        self.failed.retain(|p, _| keep.contains(p));
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        // The worker stops after its current decode: pending jobs are dropped.
        self.stop.store(true, Ordering::Relaxed);
        self.jobs = None;
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use syfrack_color::{encode::EncodedImage, write_png};
    use syfrack_core::{OutputSpace, Scene};

    fn fixture() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("syfrack-gallery-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a.state")).unwrap();
        std::fs::create_dir_all(dir.join("candidates")).unwrap();
        let mut scene = Scene::example();
        scene.seed = 42;
        let ron = scene.to_ron();
        let img = EncodedImage {
            width: 4,
            height: 2,
            bits: 8,
            data: vec![200; 4 * 2 * 3],
        };
        write_png(dir.join("a.png"), &img, OutputSpace::Srgb, Some(&ron)).unwrap();
        std::fs::write(dir.join("a.ron"), &ron).unwrap();
        std::fs::write(dir.join("a.exr"), b"not really").unwrap();
        std::fs::write(dir.join("a.state").join("state.ron"), "(scene: (seed: 1, fractal: Graph((nodes: [(id: 0, kind: Input, name: \"in\", pos: (0.0, 0.0), enabled: true), (id: 1, kind: Sphere(radius: 1.0), name: \"s\", pos: (240.0, 0.0), enabled: true), (id: 2, kind: Output, name: \"out\", pos: (480.0, 0.0), enabled: true)], links: [(from: (0, 0), to: (1, 0)), (from: (1, 0), to: (2, 0))], safety: 1.0, bounding_radius_hint: 1.0)), camera: (mode: Free(position: (0.0, 0.0, 5.0), target: (0.0, 0.0, 0.0), roll: 0.0), focal_mm: 35.0, f_number: 0.0, focus_distance: 5.0), lights: [Sun(direction: (0.0, 1.0, 0.0), color: Kelvin(6500.0), intensity: 2.0, angular_size: 1.0)], material: (albedo: (0.8, 0.8, 0.8), roughness: 0.5, metallic: 0.0, specular: 0.04, coloring: None), atmosphere: (fog_density: 0.0, fog_color: (1.0, 1.0, 1.0), fog_height: 0.0, fog_falloff: 1.0, anisotropy: 0.0, cavity_darkness: 0.0), look: (exposure_ev: 0.0, auto_exposure: None, temperature: 6500.0, tint: 0.0, shadows: (offset: (0.0, 0.0, 0.0), gain: 0.0), midtones: (offset: (0.0, 0.0, 0.0), gain: 0.0), highlights: (offset: (0.0, 0.0, 0.0), gain: 0.0), tonemap: Agx(slope: 1.0, power: 1.0, offset: 0.0), saturation: 1.0, vibrance: 0.0, vignette: None, grain: None, output: Srgb, dither: true)), cfg: (width: 64, height: 48, samples_per_pixel: 24, max_bounces: 4, max_steps: 256, tile_size: 32, samples_per_dispatch: 8, time_budget: None), tiles: [(x: 0, y: 0, w: 32, h: 32, done: 10), (x: 32, y: 0, w: 32, h: 32, done: 12), (x: 0, y: 32, w: 32, h: 16, done: 10), (x: 32, y: 32, w: 32, h: 16, done: 10)], tile_size: 32, spd: 8, dispatches: 12, elapsed_secs: 1.5)").unwrap();
        // A render without a scene and a preview that must be ignored.
        write_png(dir.join("b.png"), &img, OutputSpace::Srgb, None).unwrap();
        write_png(dir.join("b.preview.png"), &img, OutputSpace::Srgb, None).unwrap();
        write_png(
            dir.join("candidates").join("c1.png"),
            &img,
            OutputSpace::Srgb,
            None,
        )
        .unwrap();
        dir
    }

    #[test]
    fn scan_groups_files_by_stem_and_reads_state() {
        let dir = fixture();
        let entries = scan(&dir);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"a") && names.contains(&"b"));
        let a = entries.iter().find(|e| e.name == "a").unwrap();
        assert!(a.png.is_some() && a.ron.is_some() && a.exr.is_some());
        assert_eq!(a.size, Some((4, 2)));
        let st = a.state.as_ref().expect("state parsed");
        assert_eq!((st.samples_min, st.samples_max, st.target), (10, 12, 24));
        let b = entries.iter().find(|e| e.name == "b").unwrap();
        assert!(b.ron.is_none() && b.state.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dropping_the_thumbnails_returns_within_one_decode() {
        let dir = std::env::temp_dir().join(format!("syfrack-thumb-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (4000u32, 3000u32);
        let img = EncodedImage {
            width: w,
            height: h,
            bits: 8,
            data: (0..w * h * 3)
                .map(|i| (i.wrapping_mul(2654435761) >> 24) as u16)
                .collect(),
        };
        write_png(dir.join("big0.png"), &img, OutputSpace::Srgb, None).unwrap();
        let mut thumbs = Thumbnails::new();
        thumbs.request(&dir.join("big0.png"));
        for i in 1..12 {
            let p = dir.join(format!("big{i}.png"));
            std::fs::copy(dir.join("big0.png"), &p).unwrap();
            thumbs.request(&p);
        }
        let t = std::time::Instant::now();
        drop(thumbs);
        let took = t.elapsed();
        std::fs::remove_dir_all(&dir).ok();
        assert!(took < Duration::from_secs(1), "drop took {took:?}");
    }
    #[test]
    fn scene_comes_from_the_png_first_then_the_twin_ron() {
        let dir = fixture();
        let entries = scan(&dir);
        let a = entries.iter().find(|e| e.name == "a").unwrap();
        assert_eq!(scene_from_entry(a).unwrap().seed, 42);
        let b = entries.iter().find(|e| e.name == "b").unwrap();
        assert!(scene_from_entry(b).is_err(), "no scene anywhere");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn thumbnail_downscales_to_the_requested_width() {
        let dir = fixture();
        let (w, h, px) = thumbnail(&dir.join("a.png"), 2).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(px.len(), 2 * 4);
        assert_eq!(px[0], 200);
        assert_eq!(px[3], 255);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn scan_of_a_missing_dir_is_empty_not_a_panic() {
        assert!(scan(std::path::Path::new("Z:/no/such/dir")).is_empty());
    }

    #[test]
    fn a_paused_state_is_resumable_a_complete_one_is_not() {
        let dir = fixture();
        let a = scan(&dir).into_iter().find(|e| e.name == "a").unwrap();
        assert!(a.state.as_ref().unwrap().resumable());
        let path = dir.join("a.state").join("state.ron");
        let text = std::fs::read_to_string(&path).unwrap();
        // Every tile at the target of 24 samples per pixel.
        let done = text
            .replace("done: 10", "done: 24")
            .replace("done: 12", "done: 24");
        std::fs::write(&path, done).unwrap();
        let a = scan(&dir).into_iter().find(|e| e.name == "a").unwrap();
        assert!(!a.state.as_ref().unwrap().resumable());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_broken_state_file_is_skipped_not_fatal() {
        let dir = fixture();
        std::fs::write(dir.join("a.state").join("state.ron"), "(garbage").unwrap();
        let a = scan(&dir).into_iter().find(|e| e.name == "a").unwrap();
        assert!(a.state.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn thumbnail_reads_16_bit_pngs() {
        let dir = fixture();
        let img = EncodedImage {
            width: 4,
            height: 2,
            bits: 16,
            data: vec![65535; 4 * 2 * 3],
        };
        write_png(dir.join("w.png"), &img, OutputSpace::Srgb, None).unwrap();
        let (w, h, px) = thumbnail(&dir.join("w.png"), 4).unwrap();
        assert_eq!((w, h), (4, 2));
        assert_eq!(&px[..4], &[255, 255, 255, 255]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn thumbnails_bound_portraits_and_preserve_exact_landscape_dimensions() {
        let dir = fixture();
        for (width, height, limit, expected) in [
            (4, 12, 4, (1, 4)),
            (12, 4, 4, (4, 1)),
            (384, 216, 160, (160, 90)),
            (4, 12, 32, (4, 12)),
        ] {
            let image = EncodedImage {
                width,
                height,
                bits: 16,
                data: vec![65535; (width * height * 3) as usize],
            };
            let path = dir.join("bounded.png");
            write_png(&path, &image, OutputSpace::Srgb, None).unwrap();
            let (w, h, pixels) = thumbnail(&path, limit).unwrap();
            assert_eq!((w, h), expected);
            assert!(w <= limit && h <= limit);
            assert_eq!(pixels.len(), (w * h * 4) as usize);
            assert!(pixels.iter().all(|&value| value == 255));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn thumbnail_filters_linear_premultiplied_colour_and_preserves_alpha() {
        let dir = fixture();
        let path = dir.join("alpha-halo.png");
        // A red edge next to hidden green. Straight RGB averaging used to
        // create an opaque yellow fringe. The result must remain red at 50%.
        let encoded = EncodedImage {
            width: 2,
            height: 1,
            bits: 8,
            data: vec![255, 0, 0, 255, 0, 255, 0, 0],
        };
        write_png(&path, &encoded, OutputSpace::Srgb, None).unwrap();
        assert_eq!(thumbnail(&path, 1).unwrap(), (1, 1, vec![255, 0, 0, 128]));
        let transparent = EncodedImage {
            data: vec![255, 255, 255, 0, 0, 255, 0, 0],
            ..encoded
        };
        write_png(&path, &transparent, OutputSpace::Srgb, None).unwrap();
        assert_eq!(thumbnail(&path, 1).unwrap().2, vec![0; 4]);
        let grey = EncodedImage {
            width: 2,
            height: 1,
            bits: 8,
            data: vec![0, 0, 0, 255, 255, 255],
        };
        write_png(&path, &grey, OutputSpace::Srgb, None).unwrap();
        assert_eq!(thumbnail(&path, 1).unwrap().2, vec![188, 188, 188, 255]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn thumbnails_decode_in_the_background_and_cache_by_mtime() {
        let dir = fixture();
        let png = dir.join("a.png");
        let mut t = Thumbnails::new();
        t.request(&png);
        assert!(t.is_pending(&png));
        let start = std::time::Instant::now();
        while t.get(&png).is_none() && start.elapsed() < std::time::Duration::from_secs(5) {
            t.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (w, h, _) = t.get(&png).expect("decoded in the background");
        assert_eq!((*w, *h), (4, 2));
        assert!(t.stamp(&png).is_some());
        // Unchanged file: no second decode is queued.
        t.request(&png);
        assert!(!t.is_pending(&png));
        t.forget_except(&HashSet::new());
        assert!(t.get(&png).is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_removes_the_files_of_one_stem_only() {
        let dir = fixture();
        let a = scan(&dir).into_iter().find(|e| e.name == "a").unwrap();
        delete_entry(&a).unwrap();
        assert!(!dir.join("a.png").exists() && !dir.join("a.ron").exists());
        assert!(!dir.join("a.exr").exists() && !dir.join("a.state").exists());
        assert!(dir.join("b.png").exists() && dir.join("candidates").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn utc_dates_are_civil() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(
            format_utc(951_782_400 + 3600 * 13 + 60 * 5),
            "2000-02-29 13:05"
        );
        assert_eq!(format_utc(1_791_295_200), "2026-10-06 14:00");
    }

    #[test]
    fn a_one_bit_grey_png_decodes_and_never_kills_the_worker() {
        let dir = fixture();
        let path = dir.join("bw.png");
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(file, 8, 2);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0b1111_0000, 0b0000_1111]).unwrap();
        w.finish().unwrap();
        let (tw, th, px) = thumbnail(&path, 8).unwrap();
        assert_eq!((tw, th), (8, 2));
        assert_eq!(&px[..4], &[255, 255, 255, 255]);
        assert_eq!(&px[7 * 4..7 * 4 + 3], &[0, 0, 0]);
        // A corrupt file fails once, and the worker still serves the next one.
        std::fs::write(dir.join("bad.png"), b"not a png").unwrap();
        let mut t = Thumbnails::new();
        t.request(&dir.join("bad.png"));
        t.request(&path);
        let start = std::time::Instant::now();
        while t.get(&path).is_none() && start.elapsed() < std::time::Duration::from_secs(5) {
            t.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(t.get(&path).is_some());
        assert!(t.has_failed(&dir.join("bad.png")));
        t.request(&dir.join("bad.png"));
        assert!(
            !t.is_pending(&dir.join("bad.png")),
            "not retried while unchanged"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_corrupt_png_scene_falls_back_to_the_ron_and_both_errors_are_reported() {
        let dir = fixture();
        let img = EncodedImage {
            width: 4,
            height: 2,
            bits: 8,
            data: vec![0; 24],
        };
        write_png(
            dir.join("a.png"),
            &img,
            OutputSpace::Srgb,
            Some("(not a scene"),
        )
        .unwrap();
        let a = scan(&dir).into_iter().find(|e| e.name == "a").unwrap();
        let (scene, from_ron) = scene_with_source(&a).unwrap();
        assert!(from_ron);
        assert_eq!(scene.seed, 42);
        std::fs::write(dir.join("a.ron"), "(broken").unwrap();
        let err = format!("{:#}", scene_from_entry(&a).unwrap_err());
        assert!(err.contains("png") && err.contains("twin ron"), "{err}");
        std::fs::remove_dir_all(dir).ok();
    }
}
