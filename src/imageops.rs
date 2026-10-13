//! Retouches appliquées à une image avant sa conversion : réduction ou agrandissement, retrait des marges transparentes ;
//! réglages du tramage. Tout travaille sur des copies : l'image affichée n'est jamais modifiée.

use crate::model::*;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Plus grand côté, et plus grand nombre de pixels, d'une image produite : garde-fou contre une mise à l'échelle démesurée.
pub const MAX_SIDE: u64 = 32_768;
pub const MAX_PIXELS: u64 = 200_000_000;

/// Changement de taille.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resize {
    #[default]
    None,
    /// Pourcentage de la taille d'origine (1 à 1000).
    Percent(u32),
    /// Réduit l'image pour que son plus grand côté ne dépasse pas cette valeur en pixels (jamais d'agrandissement).
    MaxSide(u32),
}

/// Méthode de rééchantillonnage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resample {
    /// Lanczos, alpha prémultiplié : adapté aux photos et aux illustrations.
    #[default]
    Smooth,
    /// Plus proche voisin : conserve les pixels nets (pixel art, sprites).
    Pixel,
}

/// Algorithme de tramage pour les sorties 8 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DitherMethod {
    /// Diffusion d'erreur de Floyd-Steinberg (serpentin).
    #[default]
    FloydSteinberg,
    /// Tramage ordonné : matrice de Bayer 8 × 8.
    Bayer,
}

impl DitherMethod {
    pub fn name(self) -> &'static str {
        match self {
            DitherMethod::FloydSteinberg => "fs",
            DitherMethod::Bayer => "bayer",
        }
    }

    pub fn from_name(s: &str) -> Option<DitherMethod> {
        match s.to_ascii_lowercase().as_str() {
            "fs" | "floyd" | "floyd-steinberg" => Some(DitherMethod::FloydSteinberg),
            "bayer" | "ordered" => Some(DitherMethod::Bayer),
            _ => None,
        }
    }
}

impl Resample {
    pub fn from_name(s: &str) -> Option<Resample> {
        match s.to_ascii_lowercase().as_str() {
            "smooth" | "lanczos" => Some(Resample::Smooth),
            "pixel" | "nearest" => Some(Resample::Pixel),
            _ => None,
        }
    }
}

/// Retouches demandées avec la conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ImageOps {
    pub resize: Resize,
    pub resample: Resample,
    /// Retire les marges entièrement transparentes de chaque image.
    pub trim: bool,
    pub dither_method: DitherMethod,
}

impl ImageOps {
    /// Vrai si la conversion ne change ni la taille ni les marges de l'image.
    pub fn is_noop(&self) -> bool {
        self.resize == Resize::None && !self.trim
    }

    /// Facteur d'échelle pour une image dont le plus grand côté mesure `largest` pixels.
    pub fn scale(&self, largest: u32) -> f64 {
        match self.resize {
            Resize::None => 1.0,
            Resize::Percent(p) => p.clamp(1, 1000) as f64 / 100.0,
            Resize::MaxSide(n) if largest > n.max(1) => n.max(1) as f64 / largest as f64,
            Resize::MaxSide(_) => 1.0,
        }
    }
}

/// Rectangle (x0, y0, x1, y1) des pixels non entièrement transparents ; `None` si l'image est entièrement transparente.
pub fn opaque_bounds(rgba: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        let (Some(first), Some(last)) = (row.chunks_exact(4).position(|p| p[3] > 0), row.chunks_exact(4).rposition(|p| p[3] > 0)) else { continue };
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Copie du rectangle (x0, y0, x1, y1).
pub fn crop_rgba(rgba: &[u8], w: usize, rect: (usize, usize, usize, usize)) -> Vec<u8> {
    let (x0, y0, x1, y1) = rect;
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        out.extend_from_slice(&rgba[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    out
}

/// Redimensionne une image RGBA (non prémultipliée) en `nw` × `nh`.
pub fn resize_rgba(rgba: &[u8], w: usize, h: usize, nw: usize, nh: usize, how: Resample) -> Result<Vec<u8>> {
    use image::imageops::{FilterType, resize};
    if (w, h) == (nw, nh) {
        return Ok(rgba.to_vec());
    }
    let err = || anyhow::anyhow!("image invalide pour le redimensionnement");
    let (iw, ih, inw, inh) = (u32::try_from(w)?, u32::try_from(h)?, u32::try_from(nw)?, u32::try_from(nh)?);
    match how {
        Resample::Pixel => {
            let img = image::RgbaImage::from_raw(iw, ih, rgba.to_vec()).ok_or_else(err)?;
            Ok(resize(&img, inw, inh, FilterType::Nearest).into_raw())
        }
        Resample::Smooth => {
            // Les couleurs sont pondérées par l'alpha avant le filtrage, sinon le noir des pixels transparents « déteint » sur les bords.
            let mut pre = rgba.to_vec();
            for p in pre.chunks_exact_mut(4) {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
            let img = image::RgbaImage::from_raw(iw, ih, pre).ok_or_else(err)?;
            let mut out = resize(&img, inw, inh, FilterType::Lanczos3).into_raw();
            for p in out.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a == 0 {
                    p[..3].fill(0);
                } else if a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            Ok(out)
        }
    }
}

fn scaled(v: i32, s: f64) -> i32 {
    (v as f64 * s).round() as i32
}

/// Applique les retouches à toutes les images : renvoie une copie dont les images sont en RGBA (une image que rien ne change
/// reste telle quelle). Les positions et décalages de direction suivent la mise à l'échelle et les marges retirées, de sorte
/// que l'alignement des images d'une animation est conservé.
pub fn apply_ops(pic: &Picture, ops: &ImageOps) -> Result<Picture> {
    if ops.is_noop() {
        return Ok(pic.clone());
    }
    // Facteur commun à toutes les images : il part du plus grand côté de l'ensemble pour garder les proportions entre images.
    let largest = pic.frames.iter().map(|f| f.width.max(f.height)).max().unwrap_or(1);
    let s = ops.scale(largest);
    let mut out = pic.clone();
    for (i, f) in pic.frames.iter().enumerate() {
        let (mut w, mut h) = (f.width as usize, f.height as usize);
        let (mut px, mut py) = f.pos;
        let mut rgba: Option<Vec<u8>> = None;
        if ops.trim {
            let full = frame_to_rgba(f, pic.palette.as_ref());
            if let Some(b) = opaque_bounds(&full, w, h)
                && (b.0, b.1, b.2, b.3) != (0, 0, w, h)
            {
                rgba = Some(crop_rgba(&full, w, b));
                px += b.0 as i32;
                py += b.1 as i32;
                (w, h) = (b.2 - b.0, b.3 - b.1);
            }
        }
        if (s - 1.0).abs() > f64::EPSILON {
            let (nw, nh) = (((w as f64 * s).round() as u64).max(1), ((h as f64 * s).round() as u64).max(1));
            if nw > MAX_SIDE || nh > MAX_SIDE || nw * nh > MAX_PIXELS {
                bail!("l'image serait trop grande ({nw} × {nh})");
            }
            let src = match rgba.take() {
                Some(v) => v,
                None => frame_to_rgba(f, pic.palette.as_ref()),
            };
            rgba = Some(resize_rgba(&src, w, h, nw as usize, nh as usize, ops.resample)?);
            (w, h) = (nw as usize, nh as usize);
            (px, py) = (scaled(px, s), scaled(py, s));
        }
        if let Some(v) = rgba {
            let nf = &mut out.frames[i];
            nf.width = w as u32;
            nf.height = h as u32;
            nf.pixels = Pixels::Rgba(v);
            nf.pos = (px, py);
        }
    }
    for d in &mut out.dir_shift {
        *d = (scaled(d.0, s), scaled(d.1, s));
    }
    Ok(out)
}
