//! Sous-commandes `assemble`, `remap` et `thumbnail` de la ligne de commande.

use crate::batch;
use crate::loader::{PalPolicy, load_pal_file};
use crate::model::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Taille maximale (sur le disque) d'un fichier dont on fait une miniature.
const MAX_THUMB_FILE: u64 = 96 << 20;

fn load_target_palette(spec: &str) -> Result<Arc<Palette>, String> {
    if spec.eq_ignore_ascii_case("fallout") {
        return Ok(crate::palette::fallout());
    }
    load_pal_file(Path::new(spec)).map(Arc::new).map_err(|e| e.to_string())
}

fn number(v: Option<&String>, what: &str) -> Result<u32, String> {
    v.and_then(|v| v.parse::<u32>().ok()).ok_or_else(|| format!("{what} needs a number"))
}

pub fn run_assemble(args: &[String]) -> i32 {
    use crate::assemble::{self, AssembleOptions};
    let (mut inputs, mut out, mut pal) = (Vec::<PathBuf>::new(), None, None);
    let mut opt = AssembleOptions::default();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let parsed: Result<(), String> = match a.as_str() {
            "-o" | "--out" => {
                out = it.next().cloned();
                Ok(())
            }
            "-p" | "--pal" => {
                pal = it.next().cloned();
                Ok(())
            }
            "--directions" => number(it.next(), "--directions").map(|n| opt.directions = n as usize),
            "--fps" => number(it.next(), "--fps").map(|n| opt.fps = n),
            "--action" => number(it.next(), "--action").map(|n| opt.action_frame = n),
            "--dither" => {
                opt.dither = true;
                Ok(())
            }
            s if s.starts_with('-') => Err(format!("unknown option: {s}")),
            _ => {
                inputs.push(PathBuf::from(a));
                Ok(())
            }
        };
        if let Err(e) = parsed {
            eprintln!("error: {e}");
            return 2;
        }
    }
    let Some(out) = out else {
        eprintln!("error: usage: assemble <IMAGE>... -o <OUT.frm> [--directions N] [--fps N] [--action N] [-p PAL] [--dither]");
        return 2;
    };
    if inputs.is_empty() {
        eprintln!("error: no input image");
        return 2;
    }
    if let Some(p) = &pal {
        match load_target_palette(p) {
            Ok(p) => opt.palette = Some(p),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    let result = assemble::load_sources(&inputs)
        .and_then(|src| {
            opt.dir_shift = src.dir_shift;
            assemble::assemble_frm(src.frames, &opt)
        }).map_err(|e| e.to_string()).and_then(|bytes| {
        let out = PathBuf::from(&out);
        let dir = out.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf()).unwrap_or_default();
        let ext = out.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "frm".into());
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(&dir);
        }
        let target = batch::unique_path(&dir, &batch::stem_of(&out), &ext, Path::new(""));
        std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
        Ok(target)
    });
    match result {
        Ok(t) => {
            println!("OK    {} image file(s)  ->  {}", inputs.len(), t.display());
            0
        }
        Err(e) => {
            eprintln!("FAIL  {e}");
            1
        }
    }
}

pub fn run_remap(args: &[String]) -> i32 {
    let (mut input, mut target, mut out, mut read_pal) = (None, None, None, None);
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" | "--pal" => target = it.next().cloned(),
            "-o" | "--out" => out = it.next().cloned(),
            "--read-pal" => read_pal = it.next().cloned(),
            s if s.starts_with('-') => {
                eprintln!("error: unknown option: {s}");
                return 2;
            }
            _ => input = Some(PathBuf::from(a)),
        }
    }
    let (Some(input), Some(target)) = (input, target) else {
        eprintln!("error: usage: remap <INPUT> -p <PAL|fallout> [-o FILE_OR_DIR] [--read-pal P]");
        return 2;
    };
    let target = match load_target_palette(&target) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let policy = match read_pal.as_deref() {
        None | Some("auto") => PalPolicy::Auto,
        Some("fallout") => PalPolicy::Fallout,
        Some(f) => match load_pal_file(Path::new(f)) {
            Ok(p) => PalPolicy::Custom(Arc::new(p), f.to_string()),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
    };
    let result = crate::loader::load_file(&input, &policy).and_then(|l| {
        let r = crate::remap::remap_picture(&l.pic, &target).map_err(|e| e.to_string())?;
        let out_file = crate::remap::encode_remapped(&r).map_err(|e| e.to_string())?;
        let (bytes, ext) = (&out_file.bytes, out_file.ext);
        let with_pal = out_file.pal.is_some();
        let t = match out {
            Some(o) if !Path::new(&o).is_dir() && Path::new(&o).extension().is_some() => PathBuf::from(o),
            other => {
                let dir = match other {
                    Some(o) => PathBuf::from(o),
                    None => crate::dat::real_parent(&input).unwrap_or_default(),
                };
                let _ = std::fs::create_dir_all(&dir);
                batch::unique_path_with(&dir, &batch::stem_of(&input), ext, &input, with_pal)
            }
        };
        std::fs::write(&t, bytes).map_err(|e| e.to_string())?;
        // Un FRM n'embarque pas sa palette : le .PAL cible est écrit à côté (jamais en remplaçant un fichier existant).
        if let Some(pal) = &out_file.pal {
            let pal_path = t.with_extension("pal");
            if pal_path.exists() {
                eprintln!("note: {} already exists, the target palette was not written", pal_path.display());
            } else {
                std::fs::write(&pal_path, pal).map_err(|e| e.to_string())?;
            }
        }
        Ok(t)
    });
    match result {
        Ok(t) => {
            println!("OK    {}  ->  {}", input.display(), t.display());
            0
        }
        Err(e) => {
            eprintln!("FAIL  {}  :  {e}", input.display());
            1
        }
    }
}

pub fn run_thumbnail(args: &[String]) -> i32 {
    let (mut files, mut size) = (Vec::<String>::new(), 256u32);
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-s" | "--size" => match number(it.next(), "--size") {
                Ok(n) => size = n.clamp(8, 2048),
                Err(e) => {
                    eprintln!("error: {e}");
                    return 2;
                }
            },
            s if s.starts_with('-') => {
                eprintln!("error: unknown option: {s}");
                return 2;
            }
            _ => files.push(a.clone()),
        }
    }
    let [input, output] = files.as_slice() else {
        eprintln!("error: usage: thumbnail <INPUT> <OUT.png> [--size PX]");
        return 2;
    };
    match thumbnail_png(Path::new(input), size) {
        Ok(bytes) => match std::fs::write(output, bytes) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("FAIL  {output}  :  {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("FAIL  {input}  :  {e}");
            1
        }
    }
}

/// Vignette PNG (plus grand côté au plus `size`, sans agrandir) de la première image d'un fichier.
pub fn thumbnail_png(path: &Path, size: u32) -> Result<Vec<u8>, String> {
    // Mêmes garde-fous que les miniatures de l'application : fichier trop gros ignoré, plantage d'un décodeur sans conséquence.
    if !crate::dat::is_virtual(path) && std::fs::metadata(path).map(|m| m.len() > MAX_THUMB_FILE).unwrap_or(false) {
        return Err("file too large for a thumbnail".into());
    }
    let loaded = std::panic::catch_unwind(|| crate::loader::load_file(path, &PalPolicy::Auto)).map_err(|_| "internal error while reading the file".to_string())??;
    let pic = &loaded.pic;
    let f = pic.frames.first().ok_or("no image")?;
    let (w, h) = (f.width, f.height);
    let rgba = frame_to_rgba(f, pic.palette.as_ref());
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad image size")?;
    let img = if w.max(h) > size {
        let k = size as f32 / w.max(h) as f32;
        image::imageops::resize(&img, ((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out)
}
