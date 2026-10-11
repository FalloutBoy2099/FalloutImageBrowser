//! Conversion de fichiers vers fichiers, partagée par l'interface et la ligne de commande.

use crate::convert::*;
use crate::formats;
use crate::model::Picture;
use crate::loader::{Loaded, PalPolicy, load_file};
use std::path::{Path, PathBuf};

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

/// Prépare, encode et écrit une image ; écrit aussi le `.pal` voisin quand la palette est créée.
fn write_one(pic: &Picture, sel: &Selection, job: &Job, out_dir: &Path, stem: &str, avoid: &Path) -> Result<PathBuf, String> {
    let create_pal = matches!(job.opts.depth, DepthChoice::Indexed(PalChoice::Create));
    let prep = prepare(pic, &job.opts, sel).map_err(|e| e.to_string())?;
    let bytes = formats::encode(&prep, &job.opts.enc).map_err(|e| e.to_string())?;
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
pub fn convert_loaded(
    loaded: &Loaded,
    sel: &Selection,
    job: &Job,
    out_dir: &Path,
    name: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
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
