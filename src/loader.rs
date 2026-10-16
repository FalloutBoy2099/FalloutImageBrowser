//! Chargement asynchrone (pool de threads) avec préchargement des images voisines.

use crate::formats::{self, DecodeCtx};
use crate::model::*;
use crate::palette;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

/// Palette appliquée aux formats indexés sans palette intégrée (FRM).
#[derive(Clone)]
pub enum PalPolicy {
    /// .PAL du même nom si présent, sinon palette Fallout.
    Auto,
    Fallout,
    Custom(Arc<Palette>, String),
}

impl PalPolicy {
    pub fn same(&self, other: &PalPolicy) -> bool {
        match (self, other) {
            (PalPolicy::Auto, PalPolicy::Auto) | (PalPolicy::Fallout, PalPolicy::Fallout) => true,
            (PalPolicy::Custom(_, a), PalPolicy::Custom(_, b)) => a == b,
            _ => false,
        }
    }
}

pub struct Job {
    pub path: PathBuf,
    pub policy: PalPolicy,
    pub prefetch: bool,
}

pub struct Loaded {
    pub pic: Picture,
    pub path: PathBuf,
    pub size: u64,
    /// SHA-256 du fichier tel qu'il est stocké (aussi pour un fichier d'archive).
    pub sha256: String,
}

pub struct Done {
    pub path: PathBuf,
    pub prefetch: bool,
    pub result: Result<Arc<Loaded>, String>,
}

/// Nombre maximal de préchargements décodés en même temps : les demandes urgentes gardent toujours un thread libre, et la
/// mémoire reste raisonnable quand les images voisines sont très grandes.
const MAX_PREFETCH: usize = 2;

struct State {
    jobs: VecDeque<Job>,
    /// Préchargements en cours de décodage.
    prefetching: usize,
}

struct Queue {
    items: Mutex<State>,
    cv: Condvar,
}

pub struct Loader {
    q: Arc<Queue>,
    pub rx: Receiver<Done>,
}

impl Loader {
    pub fn new(ctx: eframe::egui::Context) -> Loader {
        let q = Arc::new(Queue { items: Mutex::new(State { jobs: VecDeque::new(), prefetching: 0 }), cv: Condvar::new() });
        let (tx, rx) = channel::<Done>();
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 4);
        for _ in 0..n {
            let q = q.clone();
            let tx: Sender<Done> = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let mut g = q.items.lock().unwrap();
                        loop {
                            // Premier travail éligible : un préchargement n'est pris que s'il reste de la place.
                            let free = g.prefetching < MAX_PREFETCH;
                            if let Some(i) = g.jobs.iter().position(|j| !j.prefetch || free) {
                                let j = g.jobs.remove(i).unwrap();
                                if j.prefetch {
                                    g.prefetching += 1;
                                }
                                break j;
                            }
                            g = q.cv.wait(g).unwrap();
                        }
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_file(&job.path, &job.policy)))
                        .unwrap_or_else(|_| Err("erreur interne lors du décodage".to_string()))
                        .map(Arc::new);
                    if job.prefetch {
                        q.items.lock().unwrap().prefetching -= 1;
                        q.cv.notify_all();
                    }
                    if tx.send(Done { path: job.path, prefetch: job.prefetch, result }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Loader { q, rx }
    }

    /// Demande urgente : passe devant les préchargements.
    pub fn request(&self, path: PathBuf, policy: PalPolicy) {
        let mut g = self.q.items.lock().unwrap();
        g.jobs.retain(|j| !j.prefetch && j.path != path);
        g.jobs.push_front(Job { path, policy, prefetch: false });
        self.q.cv.notify_one();
    }

    pub fn prefetch(&self, paths: Vec<PathBuf>, policy: &PalPolicy) {
        let mut g = self.q.items.lock().unwrap();
        g.jobs.retain(|j| !j.prefetch);
        for p in paths {
            if !g.jobs.iter().any(|j| j.path == p) {
                g.jobs.push_back(Job { path: p, policy: policy.clone(), prefetch: true });
            }
        }
        self.q.cv.notify_all();
    }
}

fn find_sibling_pal(path: &Path) -> Option<PathBuf> {
    for ext in ["pal", "PAL", "Pal"] {
        let p = path.with_extension(ext);
        if crate::dat::exists_any(&p) {
            return Some(p);
        }
    }
    None
}

pub fn load_pal_file(path: &Path) -> Result<Palette, String> {
    let data = crate::dat::read_any(path).map_err(|e| e.to_string())?;
    palette::parse_pal(&data).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_file(path: &Path, policy: &PalPolicy) -> Result<Loaded, String> {
    let data = crate::dat::read_any(path).map_err(|e| e.to_string())?;
    let by_ext = Format::from_path(path);
    let sniffed = formats::sniff(&data);
    let format = by_ext.or(sniffed).ok_or_else(|| "format non reconnu".to_string())?;

    let (pal, pal_name): (Arc<Palette>, String) = match policy {
        PalPolicy::Fallout => (palette::fallout(), "Fallout (default.pal)".into()),
        PalPolicy::Custom(p, n) => (p.clone(), n.clone()),
        PalPolicy::Auto => match find_sibling_pal(path).and_then(|p| load_pal_file(&p).ok().map(|pal| (pal, p))) {
            Some((pal, p)) => (Arc::new(pal), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            None => (palette::fallout(), "Fallout (default.pal)".into()),
        },
    };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ctx = DecodeCtx { palette: pal, palette_name: pal_name, ext };
    let mut pic = match formats::decode(format, &data, &ctx) {
        Ok(p) => p,
        Err(e) => match sniffed.filter(|s| *s != format) {
            Some(alt) => formats::decode(alt, &data, &ctx).map_err(|_| e.to_string())?,
            None => return Err(e.to_string()),
        },
    };
    let size = data.len() as u64;
    let sha256 = crate::hash::sha256_hex(&data);
    add_generic_sections(&mut pic, path, size, &sha256);
    Ok(Loaded { pic, path: path.to_path_buf(), size, sha256 })
}

fn fmt_time(t: Option<SystemTime>) -> Option<String> {
    let t = t?;
    let dt: chrono::DateTime<chrono::Local> = t.into();
    Some(dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

fn add_generic_sections(pic: &mut Picture, path: &Path, size: u64, sha256: &str) {
    let archive = crate::dat::split(path).map(|(a, _)| a);
    let meta = std::fs::metadata(archive.as_deref().unwrap_or(path)).ok();
    let mut file = Section { title: "sec.file".into(), items: Vec::new() };
    file.add("p.name", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    file.add("p.folder", path.parent().map(|p| p.display().to_string()).unwrap_or_default());
    file.add("p.size", fmt_bytes(size));
    file.add("p.sha256", sha256);
    if let Some(t) = fmt_time(meta.as_ref().and_then(|m| m.modified().ok())) {
        file.add("p.modified", t);
    }
    if let Some(t) = fmt_time(meta.as_ref().and_then(|m| m.created().ok())) {
        file.add("p.created", t);
    }
    if let Some(a) = &archive {
        file.raw("Archive", a.display().to_string());
    }
    file.add("p.format", pic.format.name());

    let mut img = Section { title: "sec.image".into(), items: Vec::new() };
    let f0 = &pic.frames[0];
    img.add("p.width", f0.width);
    img.add("p.height", f0.height);
    let px = f0.width as u64 * f0.height as u64;
    img.add("p.megapixels", format!("{:.3}", px as f64 / 1e6));
    let g = gcd(f0.width, f0.height).max(1);
    img.add("p.aspect", format!("{}:{}  ({:.4})", f0.width / g, f0.height / g, f0.width as f64 / f0.height as f64));
    img.add("p.bpp", pic.bpp);
    img.add(
        "p.color_type",
        match (&f0.pixels, pic.palette.is_some()) {
            (Pixels::Indexed(_), _) => "p.indexed",
            _ => "p.truecolor",
        },
    );
    img.add("p.transparency", if pic.has_transparency { "p.yes" } else { "p.no" });
    img.add("p.unique_colors", pic.unique_colors);
    if pic.frames.len() > 1 {
        img.add("p.frames", pic.frames.len());
    }
    pic.sections.insert(0, img);
    pic.sections.insert(0, file);
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}
