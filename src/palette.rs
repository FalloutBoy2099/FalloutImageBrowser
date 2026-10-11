//! Palettes Fallout (.PAL), quantification et diffusion d'erreur.

use crate::model::Palette;
use anyhow::{Result, bail};
use rayon::prelude::*;
use std::sync::{Arc, OnceLock};

static DEFAULT_PAL: &[u8] = include_bytes!("../assets/default.pal");

pub fn fallout() -> Arc<Palette> {
    static P: OnceLock<Arc<Palette>> = OnceLock::new();
    P.get_or_init(|| Arc::new(parse_pal(DEFAULT_PAL).expect("default.pal embarquée invalide"))).clone()
}

/// Lit un fichier .PAL (au moins 768 octets : 256 × RGB sur 6 bits, ×4 pour obtenir du 8 bits).
/// Les composantes hors plage (> 63) marquent une entrée inutilisée ; un fichier dont les
/// composantes dépassent 63 sans être 255 est interprété comme une palette 8 bits.
pub fn parse_pal(data: &[u8]) -> Result<Palette> {
    if data.len() < 768 {
        bail!("fichier PAL trop court ({} octets, 768 minimum)", data.len());
    }
    let raw = &data[..768];
    let eight_bit = raw.iter().any(|&v| v > 63 && v != 255);
    let mut p = Palette::empty();
    for i in 0..256 {
        let (r, g, b) = (raw[i * 3], raw[i * 3 + 1], raw[i * 3 + 2]);
        if eight_bit {
            p.colors[i] = [r, g, b, 255];
            p.used[i] = true;
        } else if r <= 63 && g <= 63 && b <= 63 {
            p.colors[i] = [r << 2, g << 2, b << 2, 255];
            p.used[i] = true;
        } else {
            p.colors[i] = [0, 0, 0, 255];
            p.used[i] = false;
        }
    }
    // L'index 0 est toujours transparent.
    p.colors[0][3] = 0;
    p.used[0] = true;
    Ok(p)
}

/// Palette 6 bits (0..63) depuis des composantes 8 bits : pour RIX / PAL.
pub fn to_6bit(c: u8) -> u8 {
    c >> 2
}

#[inline]
fn dist(a: [i32; 3], b: [i32; 3]) -> i32 {
    let (dr, dg, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    3 * dr * dr + 4 * dg * dg + 2 * db * db
}

/// Recherche de la couleur la plus proche via une table 64×64×64 précalculée.
pub struct Mapper {
    lut: Vec<u8>,
    pub transparent: Option<u8>,
}

impl Mapper {
    /// `grid_aligned` : les couleurs de la palette sont des multiples de 4 (palettes 6 bits) ;
    /// le centre de chaque cellule est alors sa borne basse, ce qui garantit une correspondance
    /// exacte pour les couleurs déjà présentes dans la palette.
    pub fn new(pal: &Palette, transparent: Option<u8>, grid_aligned: bool) -> Result<Mapper> {
        let cands: Vec<(u8, [i32; 3])> = (0..256usize)
            .filter(|&i| pal.used[i] && Some(i as u8) != transparent && pal.colors[i][3] > 0)
            .map(|i| (i as u8, [pal.colors[i][0] as i32, pal.colors[i][1] as i32, pal.colors[i][2] as i32]))
            .collect();
        if cands.is_empty() {
            bail!("palette vide");
        }
        let off = if grid_aligned { 0 } else { 2 };
        let mut lut = vec![0u8; 64 * 64 * 64];
        lut.par_chunks_mut(64 * 64).enumerate().for_each(|(r, plane)| {
            for g in 0..64usize {
                for b in 0..64usize {
                    let c = [((r << 2) + off) as i32, ((g << 2) + off) as i32, ((b << 2) + off) as i32];
                    let mut best = (i32::MAX, 0u8);
                    for &(i, pc) in &cands {
                        let d = dist(c, pc);
                        if d < best.0 {
                            best = (d, i);
                            if d == 0 {
                                break;
                            }
                        }
                    }
                    plane[g * 64 + b] = best.1;
                }
            }
        });
        Ok(Mapper { lut, transparent })
    }

    #[inline]
    pub fn nearest(&self, r: u8, g: u8, b: u8) -> u8 {
        self.lut[((r as usize >> 2) << 12) | ((g as usize >> 2) << 6) | (b as usize >> 2)]
    }
}

/// Table de correspondance index source -> index cible pour une image déjà indexée.
pub fn remap_table(src: &Palette, dst: &Palette, mapper: &Mapper) -> [u8; 256] {
    let mut t = [0u8; 256];
    for i in 0..256usize {
        let c = src.colors[i];
        t[i] = if let (true, Some(tr)) = (c[3] == 0, mapper.transparent) {
            tr
        } else if !src.used[i] {
            // Entrée inutilisée dans les deux palettes (ex. couleurs animées 229-254) : index conservé.
            if !dst.used[i] {
                i as u8
            } else {
                mapper.transparent.unwrap_or_else(|| mapper.nearest(0, 0, 0))
            }
        } else if dst.used[i]
            && dst.colors[i][3] > 0
            && Some(i as u8) != mapper.transparent
            && dst.colors[i][..3] == c[..3]
        {
            i as u8
        } else {
            mapper.nearest(c[0], c[1], c[2])
        };
    }
    t
}

/// Remplace les entrées transparentes par une couleur de fond opaque.
pub fn flatten_palette(p: &Palette, bg: [u8; 3]) -> Palette {
    let mut q = p.clone();
    for i in 0..256 {
        if q.colors[i][3] < 128 {
            q.colors[i] = [bg[0], bg[1], bg[2], 255];
        } else {
            q.colors[i][3] = 255;
        }
    }
    q
}

/// Quantifie un buffer RGBA vers une palette (avec diffusion d'erreur de Floyd-Steinberg optionnelle).
/// Les pixels dont alpha < 128 deviennent `mapper.transparent` s'il existe (sinon ils doivent
/// avoir été aplatis au préalable).
pub fn quantize_rgba(rgba: &[u8], w: usize, h: usize, pal: &Palette, mapper: &Mapper, dither: bool) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    if !dither {
        out.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                row[x] = if p[3] < 128 && mapper.transparent.is_some() {
                    mapper.transparent.unwrap()
                } else {
                    mapper.nearest(p[0], p[1], p[2])
                };
            }
        });
        return out;
    }
    // Floyd-Steinberg, serpentin.
    let mut cur = vec![[0i32; 3]; w + 2];
    let mut nxt = vec![[0i32; 3]; w + 2];
    for y in 0..h {
        for e in nxt.iter_mut() {
            *e = [0; 3];
        }
        let ltr = y % 2 == 0;
        for step in 0..w {
            let x = if ltr { step } else { w - 1 - step };
            let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            if p[3] < 128 && mapper.transparent.is_some() {
                out[y * w + x] = mapper.transparent.unwrap();
                continue;
            }
            let e = cur[x + 1];
            let c = [
                (p[0] as i32 + e[0] / 16).clamp(0, 255),
                (p[1] as i32 + e[1] / 16).clamp(0, 255),
                (p[2] as i32 + e[2] / 16).clamp(0, 255),
            ];
            let i = mapper.nearest(c[0] as u8, c[1] as u8, c[2] as u8);
            out[y * w + x] = i;
            let q = pal.colors[i as usize];
            let err = [c[0] - q[0] as i32, c[1] - q[1] as i32, c[2] - q[2] as i32];
            let (fx, bx) = if ltr { (x + 2, x) } else { (x, x + 2) };
            for k in 0..3 {
                cur[fx][k] += err[k] * 7;
                nxt[bx][k] += err[k] * 3;
                nxt[x + 1][k] += err[k] * 5;
                nxt[fx][k] += err[k];
            }
        }
        std::mem::swap(&mut cur, &mut nxt);
    }
    out
}

/// Construit une palette adaptative (≤ `max_colors` couleurs). Si l'ensemble des couleurs
/// opaques tient dans la palette, elle est exacte. `reserve_transparent` réserve l'index 0.
pub fn adaptive_palette(frames: &[&[u8]], reserve_transparent: bool, bg: [u8; 3]) -> Palette {
    let max_colors = if reserve_transparent { 255 } else { 256 };
    let mut set: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut overflow = false;
    'outer: for f in frames {
        for p in f.chunks_exact(4) {
            if p[3] < 128 {
                continue;
            }
            set.insert(u32::from_le_bytes([p[0], p[1], p[2], 0]));
            if set.len() > max_colors {
                overflow = true;
                break 'outer;
            }
        }
    }
    let base = if reserve_transparent { 1 } else { 0 };
    let mut pal = Palette::empty();
    let mut colors: Vec<[u8; 3]> = Vec::new();
    if !overflow {
        let mut v: Vec<u32> = set.into_iter().collect();
        v.sort_unstable();
        for k in v {
            let b = k.to_le_bytes();
            colors.push([b[0], b[1], b[2]]);
        }
    } else {
        // Échantillonnage des pixels opaques pour NeuQuant.
        let total: usize = frames.iter().map(|f| f.len() / 4).sum();
        let step = (total / 300_000).max(1);
        let mut sample: Vec<u8> = Vec::with_capacity(total / step * 4 + 4);
        let mut n = 0usize;
        for f in frames {
            for p in f.chunks_exact(4) {
                if p[3] < 128 {
                    continue;
                }
                if n % step == 0 {
                    sample.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
                n += 1;
            }
        }
        let nq = color_quant::NeuQuant::new(if total > 2_000_000 { 10 } else { 3 }, max_colors, &sample);
        let map = nq.color_map_rgba();
        for c in map.chunks_exact(4) {
            colors.push([c[0], c[1], c[2]]);
        }
    }
    if reserve_transparent {
        pal.colors[0] = [bg[0], bg[1], bg[2], 0];
        pal.used[0] = true;
    }
    for (k, c) in colors.iter().enumerate().take(max_colors) {
        pal.colors[base + k] = [c[0], c[1], c[2], 255];
        pal.used[base + k] = true;
    }
    if colors.is_empty() && !reserve_transparent {
        pal.colors[0] = [0, 0, 0, 255];
        pal.used[0] = true;
    }
    pal
}

/// Écriture d'une palette .PAL 6 bits (768 octets).
pub fn write_pal(p: &Palette) -> Vec<u8> {
    let mut v = Vec::with_capacity(768);
    for i in 0..256 {
        let c = if p.used[i] { p.colors[i] } else { [0, 0, 0, 255] };
        v.extend_from_slice(&[c[0] >> 2, c[1] >> 2, c[2] >> 2]);
    }
    v
}

/// Aligne les couleurs sur la grille 6 bits d'un fichier .PAL (multiples de 4).
pub fn snap_6bit(p: &mut Palette) {
    for i in 1..256 {
        for c in 0..3 {
            p.colors[i][c] &= 0xFC;
        }
    }
}
