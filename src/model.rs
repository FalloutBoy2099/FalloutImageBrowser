//! Modèle de données commun à tous les formats.

use rayon::prelude::*;
use std::borrow::Cow;
use std::path::Path;

/// Palette de 256 entrées RGBA. `alpha == 0` = entrée transparente.
/// `used == false` = entrée inutilisée (ignorée lors de la quantification).
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub colors: [[u8; 4]; 256],
    pub used: [bool; 256],
}

impl Palette {
    pub fn empty() -> Self {
        Palette { colors: [[0, 0, 0, 255]; 256], used: [false; 256] }
    }

    pub fn count_used(&self) -> usize {
        self.used.iter().filter(|u| **u).count()
    }

    #[inline]
    pub fn rgba(&self, idx: u8) -> [u8; 4] {
        self.colors[idx as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Art,
    Bmp,
    Frm,
    Gif,
    Jpeg,
    Pcx,
    Png,
    Rix,
    Tiff,
    Tga,
    Webp,
    // Formats en lecture seule (image fusionnée, sans calques).
    Psd,
    Pdn,
    Xcf,
    Kra,
    Ora,
    Dds,
    Qoi,
    Pnm,
    // Lecture seule (suite) : Aseprite et fichiers de palette (nuancier).
    Ase,
    Palette,
    // Fallout Tactics (lecture seule).
    Zar,
    Til,
    Spr,
}

impl Format {
    /// Formats lisibles ET inscriptibles (cibles de conversion).
    pub const ALL: [Format; 14] = [
        Format::Art,
        Format::Bmp,
        Format::Dds,
        Format::Frm,
        Format::Gif,
        Format::Jpeg,
        Format::Pcx,
        Format::Png,
        Format::Pnm,
        Format::Qoi,
        Format::Rix,
        Format::Tiff,
        Format::Tga,
        Format::Webp,
    ];

    /// Formats en lecture seule.
    pub const READ_ONLY: [Format; 10] = [Format::Psd, Format::Pdn, Format::Xcf, Format::Kra, Format::Ora, Format::Ase, Format::Palette, Format::Zar, Format::Til, Format::Spr];

    pub fn is_writable(self) -> bool {
        !Self::READ_ONLY.contains(&self)
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Art => "ART",
            Format::Bmp => "BMP",
            Format::Frm => "FRM",
            Format::Gif => "GIF",
            Format::Jpeg => "JPEG",
            Format::Pcx => "PCX",
            Format::Png => "PNG",
            Format::Rix => "RIX",
            Format::Tiff => "TIFF",
            Format::Tga => "TGA",
            Format::Webp => "WEBP",
            Format::Psd => "PSD",
            Format::Pdn => "PDN",
            Format::Xcf => "XCF",
            Format::Kra => "KRA",
            Format::Ora => "ORA",
            Format::Dds => "DDS",
            Format::Qoi => "QOI",
            Format::Pnm => "PNM",
            Format::Ase => "ASE",
            Format::Palette => "PALETTE",
            Format::Zar => "ZAR",
            Format::Til => "TIL",
            Format::Spr => "SPR",
        }
    }

    /// Extension utilisée à l'écriture.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Art => "art",
            Format::Bmp => "bmp",
            Format::Frm => "frm",
            Format::Gif => "gif",
            Format::Jpeg => "jpg",
            Format::Pcx => "pcx",
            Format::Png => "png",
            Format::Rix => "rix",
            Format::Tiff => "tif",
            Format::Tga => "tga",
            Format::Webp => "webp",
            Format::Psd => "psd",
            Format::Pdn => "pdn",
            Format::Xcf => "xcf",
            Format::Kra => "kra",
            Format::Ora => "ora",
            Format::Dds => "dds",
            Format::Qoi => "qoi",
            Format::Pnm => "ppm",
            Format::Ase => "ase",
            Format::Palette => "pal",
            Format::Zar => "zar",
            Format::Til => "til",
            Format::Spr => "spr",
        }
    }

    pub fn from_ext(ext: &str) -> Option<Format> {
        let e = ext.to_ascii_lowercase();
        Some(match e.as_str() {
            "art" => Format::Art,
            "bmp" | "dib" => Format::Bmp,
            "frm" | "fr0" | "fr1" | "fr2" | "fr3" | "fr4" | "fr5" => Format::Frm,
            "gif" => Format::Gif,
            "jpg" | "jpeg" | "jpe" | "jfif" => Format::Jpeg,
            "pcx" => Format::Pcx,
            "png" => Format::Png,
            "rix" => Format::Rix,
            "tif" | "tiff" => Format::Tiff,
            "tga" | "targa" => Format::Tga,
            "webp" => Format::Webp,
            "psd" | "psb" => Format::Psd,
            "pdn" => Format::Pdn,
            "xcf" => Format::Xcf,
            "kra" => Format::Kra,
            "ora" => Format::Ora,
            "dds" => Format::Dds,
            "qoi" => Format::Qoi,
            "pbm" | "pgm" | "ppm" | "pnm" | "pam" => Format::Pnm,
            "ase" | "aseprite" => Format::Ase,
            "pal" | "act" | "gpl" | "hex" | "kpl" => Format::Palette,
            "zar" => Format::Zar,
            "til" => Format::Til,
            "spr" => Format::Spr,
            _ => return None,
        })
    }

    pub fn from_path(p: &Path) -> Option<Format> {
        Format::from_ext(p.extension()?.to_str()?)
    }
}

pub fn is_supported(p: &Path) -> bool {
    Format::from_path(p).is_some()
}

#[derive(Clone, Debug)]
pub enum Pixels {
    /// Un octet par pixel (index dans `Picture::palette`).
    Indexed(Vec<u8>),
    /// RGBA 8 bits, alpha non prémultiplié.
    Rgba(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    /// Position du coin haut-gauche par rapport à l'origine commune de l'animation.
    pub pos: (i32, i32),
    pub delay_ms: Option<u32>,
}

/// Une ligne de la fenêtre « Propriétés ». `key` est une clé de traduction
/// (ou un texte brut s'il commence par '@').
#[derive(Clone, Debug)]
pub struct Prop {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct Section {
    pub title: String,
    pub items: Vec<Prop>,
}

#[derive(Clone, Debug)]
pub struct Picture {
    pub format: Format,
    pub frames: Vec<Frame>,
    pub directions: usize,
    pub frames_per_dir: usize,
    pub palette: Option<Palette>,
    pub fps: u32,
    pub action_frame: u32,
    pub dir_shift: Vec<(i32, i32)>,
    /// Libellé de chaque direction (vide : numéros) — ex. les animations d'un SPR Fallout Tactics.
    pub dir_labels: Vec<String>,
    /// Nombre d'images réelles de chaque direction (vide : `frames_per_dir` partout).
    pub dir_counts: Vec<usize>,
    /// Profondeur d'origine (bits par pixel) du fichier.
    pub bpp: u32,
    pub sections: Vec<Section>,
    pub unique_colors: usize,
    pub has_transparency: bool,
    /// Palette d'un fichier de palette affiché en nuancier (l'image est alors un aperçu RVBA).
    pub colors: Option<Palette>,
}

impl Picture {
    pub fn new(format: Format) -> Self {
        Picture {
            format,
            frames: Vec::new(),
            directions: 1,
            frames_per_dir: 1,
            palette: None,
            fps: 0,
            action_frame: 0,
            dir_shift: vec![(0, 0)],
            dir_labels: Vec::new(),
            dir_counts: Vec::new(),
            bpp: 0,
            sections: Vec::new(),
            unique_colors: 0,
            has_transparency: false,
            colors: None,
        }
    }

    pub fn single(format: Format, width: u32, height: u32, pixels: Pixels, palette: Option<Palette>) -> Self {
        let mut p = Picture::new(format);
        p.frames.push(Frame { width, height, pixels, pos: (0, 0), delay_ms: None });
        p.palette = palette;
        p
    }

    pub fn frame_index(&self, dir: usize, f: usize) -> usize {
        dir.min(self.directions.saturating_sub(1)) * self.frames_per_dir + f.min(self.frames_per_dir.saturating_sub(1))
    }

    pub fn dir_frames(&self, dir: usize) -> &[Frame] {
        let d = dir.min(self.directions.saturating_sub(1));
        let s = d * self.frames_per_dir;
        let e = (s + self.frames_per_dir).min(self.frames.len());
        &self.frames[s.min(e)..e]
    }

    /// Rectangle englobant (min_x, min_y, max_x, max_y) de toutes les images d'une direction.
    pub fn dir_bounds(&self, dir: usize) -> (i32, i32, i32, i32) {
        let fr = self.dir_frames(dir);
        if fr.is_empty() {
            return (0, 0, 1, 1);
        }
        let mut b = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for f in fr {
            b.0 = b.0.min(f.pos.0);
            b.1 = b.1.min(f.pos.1);
            b.2 = b.2.max(f.pos.0 + f.width as i32);
            b.3 = b.3.max(f.pos.1 + f.height as i32);
        }
        b
    }

    /// Nombre d'images réelles de la direction `dir`.
    pub fn dir_len(&self, dir: usize) -> usize {
        self.dir_counts.get(dir).copied().unwrap_or(self.frames_per_dir).clamp(1, self.frames_per_dir.max(1))
    }

    pub fn dir_label(&self, dir: usize) -> String {
        self.dir_labels.get(dir).cloned().unwrap_or_else(|| format!("{}", dir + 1))
    }

    pub fn is_animated(&self) -> bool {
        self.frames_per_dir > 1
    }

    pub fn section(&mut self, title: &str) -> &mut Section {
        self.sections.push(Section { title: title.to_string(), items: Vec::new() });
        self.sections.last_mut().unwrap()
    }
}

impl Section {
    pub fn add(&mut self, key: &str, value: impl ToString) {
        self.items.push(Prop { key: key.to_string(), value: value.to_string() });
    }
    pub fn raw(&mut self, name: &str, value: impl ToString) {
        self.items.push(Prop { key: format!("@{name}"), value: value.to_string() });
    }
}

/// Rendu RGBA d'une image (résolution de la palette incluse).
pub fn frame_to_rgba(frame: &Frame, palette: Option<&Palette>) -> Vec<u8> {
    match &frame.pixels {
        Pixels::Rgba(v) => v.clone(),
        Pixels::Indexed(idx) => {
            let mut lut = [[0u8; 4]; 256];
            if let Some(p) = palette {
                for i in 0..256 {
                    lut[i] = if p.used[i] { p.colors[i] } else { [0, 0, 0, 255] };
                }
            } else {
                for i in 0..256 {
                    lut[i] = [i as u8, i as u8, i as u8, 255];
                }
            }
            let mut out = vec![0u8; idx.len() * 4];
            // Les grandes images sont converties en parallèle, par blocs.
            const BLOCK: usize = 1 << 16;
            let fill = |dst: &mut [u8], src: &[u8]| {
                for (d, &i) in dst.chunks_exact_mut(4).zip(src) {
                    d.copy_from_slice(&lut[i as usize]);
                }
            };
            if idx.len() < 4 * BLOCK {
                fill(&mut out, idx);
            } else {
                out.par_chunks_mut(BLOCK * 4).zip(idx.par_chunks(BLOCK)).for_each(|(d, s)| fill(d, s));
            }
            out
        }
    }
}

/// Comme `frame_to_rgba`, sans copie quand l'image est déjà en RGBA.
pub fn frame_to_rgba_cow<'a>(frame: &'a Frame, palette: Option<&Palette>) -> Cow<'a, [u8]> {
    match &frame.pixels {
        Pixels::Rgba(v) => Cow::Borrowed(v),
        Pixels::Indexed(_) => Cow::Owned(frame_to_rgba(frame, palette)),
    }
}

pub fn fmt_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KiB ({n} B)", n as f64 / 1024.0)
    } else {
        format!("{:.2} MiB ({n} B)", n as f64 / 1048576.0)
    }
}

/// Compte les couleurs distinctes (RGB, alpha > 0 ignoré) d'un buffer RGBA, plafonné.
pub fn count_unique_rgb(rgba: &[u8], seen: &mut Vec<u64>) -> usize {
    if seen.is_empty() {
        seen.resize((1 << 24) / 64, 0);
    }
    let mut n = 0usize;
    for px in rgba.chunks_exact(4) {
        let k = ((px[0] as usize) << 16) | ((px[1] as usize) << 8) | px[2] as usize;
        let (w, b) = (k >> 6, k & 63);
        if seen[w] & (1u64 << b) == 0 {
            seen[w] |= 1u64 << b;
            n += 1;
        }
    }
    n
}
