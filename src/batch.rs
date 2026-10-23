//! Conversion de fichiers vers fichiers, partagée par l'interface et la ligne de commande.

use crate::convert::*;
use crate::formats::{self, EncodeOpts};
use crate::imageops::ImageOps;
use crate::loader::{Loaded, PalPolicy, load_file};
use crate::model::{Format, Frame, Picture, Pixels, frame_to_rgba, is_supported};
use crate::tree::natural_cmp;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct Job {
    pub opts: ConvertOptions,
    /// Exporter toutes les images (séquence) quand le format cible n'en contient qu'une.
    pub seq: bool,
    pub policy: PalPolicy,
}

/// Chemin de sortie libre : ne remplace jamais un fichier existant ni le fichier source.
pub fn unique_path(dir: &Path, stem: &str, ext: &str, avoid: &Path) -> PathBuf {
    unique_path_with(dir, stem, ext, avoid, false)
}

/// Comme `unique_path`, en exigeant aussi que le `.pal` de même nom soit libre (palette créée à côté de l'image).
pub fn unique_path_with(dir: &Path, stem: &str, ext: &str, avoid: &Path, with_pal: bool) -> PathBuf {
    let taken = |p: &Path| p.exists() || p == avoid || (with_pal && p.with_extension("pal").exists());
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 0;
    while taken(&p) {
        n += 1;
        p = if n == 1 { dir.join(format!("{stem}_converted.{ext}")) } else { dir.join(format!("{stem}_converted_{n}.{ext}")) };
    }
    p
}

/// Choix d'un nom libre puis écriture : un seul thread à la fois, pour que deux conversions parallèles ne choisissent pas le même nom.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Prépare, encode et écrit une image ; écrit aussi le `.pal` voisin quand la palette est créée.
fn write_one(pic: &Picture, sel: &Selection, job: &Job, out_dir: &Path, stem: &str, avoid: &Path) -> Result<PathBuf, String> {
    let create_pal = matches!(job.opts.depth, DepthChoice::Indexed(PalChoice::Create));
    let prep = prepare(pic, &job.opts, sel).map_err(|e| e.to_string())?;
    let bytes = formats::encode(&prep, &job.opts.enc).map_err(|e| e.to_string())?;
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = unique_path_with(out_dir, stem, job.opts.format.ext(), avoid, create_pal);
    std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
    if let (true, Some(pal)) = (create_pal, &prep.palette) {
        std::fs::write(out.with_extension("pal"), crate::palette::write_pal(pal)).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn stem_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "image".into())
}

/// Convertit une image déjà chargée. `name` remplace le nom (sans extension) du fichier d'origine.
pub fn convert_loaded(loaded: &Loaded, sel: &Selection, job: &Job, out_dir: &Path, name: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let pic = &loaded.pic;
    let fmt = job.opts.format;
    let caps = formats::caps(fmt);
    let stem = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => stem_of(&loaded.path),
    };
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut written = Vec::new();
    if job.seq && !caps.multi_frame && pic.frames.len() > 1 {
        for d in 0..pic.directions {
            for f in 0..pic.frames_per_dir {
                let one = single_frame(pic, d, f);
                let n = if pic.directions > 1 { format!("{stem}_d{d}_{:03}", f + 1) } else { format!("{stem}_{:03}", f + 1) };
                let out = write_one(&one, &Selection::default(), job, out_dir, &n, &loaded.path)?;
                written.push(out);
            }
        }
    } else {
        let out = write_one(pic, sel, job, out_dir, &stem, &loaded.path)?;
        written.push(out);
    }
    Ok(written)
}

/// Charge puis convertit un fichier (`sel` : image / direction exportées pour les formats à image unique).
pub fn convert_path(path: &Path, job: &Job, out_dir: &Path, name: Option<&str>, sel: &Selection) -> Result<Vec<PathBuf>, String> {
    let loaded = load_file(path, &job.policy)?;
    convert_loaded(&loaded, sel, job, out_dir, name)
}

/// Image convertible en mode répertoire (les fichiers de palette sont exclus).
pub fn convertible(p: &Path) -> bool {
    is_supported(p) && Format::from_path(p) != Some(Format::Palette)
}

/// Chemin de sortie d'une animation : `name` (sans extension) dans `out_dir`, avec l'extension du format.
pub fn animation_path(out_dir: &Path, name: &str, format: Format) -> PathBuf {
    out_dir.join(format!("{}.{}", name.trim(), format.ext()))
}

/// Crée une animation FRM ou ART à partir des images de `dir`, prises dans l'ordre naturel des noms (première image de
/// chaque fichier), et l'écrit sous `name` (sans extension) dans `out_dir`. Ne remplace jamais un fichier existant.
/// Renvoie le chemin écrit et le nombre d'images.
pub fn build_animation(dir: &Path, out_dir: &Path, name: &str, format: Format, policy: &PalPolicy) -> Result<(PathBuf, usize), String> {
    crate::fileops::validate_name(name.trim()).map_err(|e| e.to_string())?;
    let out = animation_path(out_dir, name, format);
    if out.exists() {
        return Err(crate::i18n::tr("t.rename_exists"));
    }
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && convertible(p) && !matches!(Format::from_path(p), Some(Format::Frm | Format::Art)))
        .collect();
    inputs.sort_by(|a, b| natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    if inputs.is_empty() {
        return Err(crate::i18n::tr("t.anim_none"));
    }
    let frames: Vec<Frame> = inputs
        .par_iter()
        .map(|p| {
            let loaded = load_file(p, policy)?;
            let f = loaded.pic.frames.first().ok_or_else(|| format!("{}: image vide", p.display()))?;
            let rgba = frame_to_rgba(f, loaded.pic.palette.as_ref());
            // Toutes les images sont ancrées au centre du bord bas (convention des animations Fallout).
            Ok(Frame { width: f.width, height: f.height, pixels: Pixels::Rgba(rgba), pos: (-(f.width as i32 / 2), -(f.height as i32)), delay_ms: None })
        })
        .collect::<Result<_, String>>()?;
    let n = frames.len();
    let mut pic = Picture::new(Format::Png);
    pic.frames = frames;
    pic.frames_per_dir = n;
    pic.fps = 10;
    pic.has_transparency = true;
    let pal = match (format, policy) {
        (Format::Frm, PalPolicy::Custom(p, _)) => PalChoice::Custom(p.clone()),
        (Format::Frm, _) => PalChoice::Fallout,
        _ => PalChoice::Adaptive,
    };
    let opts = ConvertOptions { format, depth: DepthChoice::Indexed(pal), dither: false, bg: [0, 0, 0], enc: EncodeOpts::default(), ops: ImageOps::default(), text_white: false };
    let prep = prepare(&pic, &opts, &Selection::default()).map_err(|e| e.to_string())?;
    let bytes = formats::encode(&prep, &opts.enc).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
    Ok((out, n))
}

/// (chemin source, sous-dossier relatif à conserver sous le dossier de sortie), mode répertoire, depuis une archive
pub fn list_inputs(input: &Path, recursive: bool) -> Result<(Vec<(PathBuf, PathBuf)>, bool, bool), String> {
    // Archive ou dossier virtuel.
    if let Some((_, inner)) = crate::dat::split(input) {
        if crate::dat::stat(input) == Some(crate::dat::Stat::Dir) || inner.is_empty() {
            let whole_archive = inner.is_empty();
            let v: Vec<_> = crate::dat::walk(input)
                .into_iter()
                .filter(|(p, rel)| convertible(p) && (whole_archive || recursive || rel.components().count() == 1))
                .map(|(p, rel)| {
                    let parent = rel.parent().map(|r| r.to_path_buf()).unwrap_or_default();
                    (p, parent)
                })
                .collect();
            return Ok((v, true, true));
        }
        return Ok((vec![(input.to_path_buf(), PathBuf::new())], false, true));
    }
    if input.is_file() {
        return Ok((vec![(input.to_path_buf(), PathBuf::new())], false, false));
    }
    if input.is_dir() {
        let mut out = Vec::new();
        fn walk(dir: &Path, rel: &Path, recursive: bool, out: &mut Vec<(PathBuf, PathBuf)>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let p = e.path();
                if p.is_dir() {
                    if recursive {
                        walk(&p, &rel.join(e.file_name()), recursive, out);
                    }
                } else if convertible(&p) {
                    out.push((p, rel.to_path_buf()));
                }
            }
        }
        walk(input, Path::new(""), recursive, &mut out);
        return Ok((out, true, false));
    }
    Err(format!("not found: {}", input.display()))
}

/// Un fichier à convertir et le sous-dossier (relatif au dossier ajouté) à conserver sous le dossier de sortie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchItem {
    pub path: PathBuf,
    pub rel: PathBuf,
}

/// Où écrire les fichiers d'un lot.
#[derive(Clone, Debug)]
pub enum BatchOut {
    /// À côté de chaque fichier source (pour une archive : à côté de l'archive).
    NextToSource,
    /// Dans un dossier ; `keep_tree` recrée les sous-dossiers des fichiers ajoutés avec un dossier.
    Folder { dir: PathBuf, keep_tree: bool },
}

/// Avancement d'un lot, lu par l'interface pendant que le travail se fait dans d'autres threads.
#[derive(Default)]
pub struct Progress {
    pub total: AtomicUsize,
    /// Fichiers traités (réussis ou non).
    pub done: AtomicUsize,
    pub failed: AtomicUsize,
    /// Fichiers écrits (une animation exportée en séquence en produit plusieurs).
    pub written: AtomicUsize,
    pub cancel: AtomicBool,
    pub finished: AtomicBool,
    pub errors: Mutex<Vec<String>>,
}

/// Erreurs conservées au plus : un lot de milliers de fichiers identiques ne doit pas saturer la mémoire.
const MAX_ERRORS: usize = 200;

impl Progress {
    pub fn new(total: usize) -> Progress {
        let p = Progress::default();
        p.total.store(total, Ordering::Relaxed);
        p
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Dossier de sortie d'un fichier du lot.
pub fn item_out_dir(item: &BatchItem, out: &BatchOut) -> PathBuf {
    match out {
        BatchOut::NextToSource => crate::dat::real_parent(&item.path).unwrap_or_default(),
        BatchOut::Folder { dir, keep_tree: true } => dir.join(&item.rel),
        BatchOut::Folder { dir, keep_tree: false } => dir.clone(),
    }
}

/// Convertit tous les fichiers en parallèle ; s'arrête (entre deux fichiers) si `progress.cancel` est levé. Aucun fichier
/// existant n'est remplacé. Les échecs sont comptés et décrits dans `progress`, sans interrompre le lot.
pub fn run_batch(items: &[BatchItem], job: &Job, out: &BatchOut, sel: &Selection, progress: &Progress) {
    items.par_iter().for_each(|item| {
        if progress.is_cancelled() {
            return;
        }
        let out_dir = item_out_dir(item, out);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| convert_path(&item.path, job, &out_dir, None, sel))).unwrap_or_else(|_| Err("internal error".into()));
        match res {
            Ok(files) => {
                progress.written.fetch_add(files.len(), Ordering::Relaxed);
            }
            Err(e) => {
                progress.failed.fetch_add(1, Ordering::Relaxed);
                let mut errs = progress.errors.lock().unwrap_or_else(|e| e.into_inner());
                if errs.len() < MAX_ERRORS {
                    errs.push(format!("{} : {e}", item.path.display()));
                }
            }
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
    });
    progress.finished.store(true, Ordering::Release);
}
