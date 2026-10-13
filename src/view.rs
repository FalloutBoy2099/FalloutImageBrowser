//! Calculs d'affichage sans interface (donc testables) : canevas d'une direction, rotation / symétries de la vue,
//! lecture d'un pixel.

use crate::model::*;
use rayon::prelude::*;
use std::borrow::Cow;

/// Taille logique (canevas commun d'une direction pour les animations).
pub fn canvas_size(pic: &Picture, dir: usize) -> (u32, u32) {
    if pic.frames_per_dir > 1 {
        let (x0, y0, x1, y1) = pic.dir_bounds(dir);
        let (w, h) = ((x1 - x0) as i64, (y1 - y0) as i64);
        if w > 0 && h > 0 && w * h <= 64_000_000 {
            return (w as u32, h as u32);
        }
    }
    let f = &pic.frames[pic.frame_index(dir, 0)];
    (f.width, f.height)
}

/// Décalage de l'image `frame` dans le canevas de la direction (0, 0 pour une image seule ou qui remplit le canevas).
fn canvas_offset(pic: &Picture, dir: usize, frame: &Frame) -> (i32, i32) {
    let (cw, ch) = canvas_size(pic, dir);
    if pic.frames_per_dir <= 1 || (cw == frame.width && ch == frame.height) {
        return (0, 0);
    }
    let (x0, y0, _, _) = pic.dir_bounds(dir);
    (frame.pos.0 - x0, frame.pos.1 - y0)
}

/// Pixels RGBA du canevas de la direction `dir` pour l'image `f` : (largeur, hauteur, pixels).
pub fn render_rgba(pic: &Picture, dir: usize, f: usize) -> (usize, usize, Vec<u8>) {
    let (w, h, rgba) = render_rgba_cow(pic, dir, f);
    (w, h, rgba.into_owned())
}

/// Comme `render_rgba`, sans copier les pixels d'une image RGBA qui remplit déjà le canevas.
pub fn render_rgba_cow(pic: &Picture, dir: usize, f: usize) -> (usize, usize, Cow<'_, [u8]>) {
    let frame = &pic.frames[pic.frame_index(dir, f)];
    let (cw, ch) = canvas_size(pic, dir);
    let rgba = frame_to_rgba_cow(frame, pic.palette.as_ref());
    if pic.frames_per_dir <= 1 || (cw == frame.width && ch == frame.height) {
        return (frame.width as usize, frame.height as usize, rgba);
    }
    let (ox, oy) = canvas_offset(pic, dir, frame);
    let (ox, oy) = (ox as usize, oy as usize);
    let (cw, ch) = (cw as usize, ch as usize);
    let mut canvas = vec![0u8; cw * ch * 4];
    for y in 0..frame.height as usize {
        let dst = ((oy + y) * cw + ox) * 4;
        let src = y * frame.width as usize * 4;
        let n = frame.width as usize * 4;
        if dst + n <= canvas.len() {
            canvas[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
        }
    }
    (cw, ch, Cow::Owned(canvas))
}

// ----------------------------------------------------------------------------- toutes les directions côte à côte

/// Nombre maximal de directions affichées ensemble (au-delà, une grille serait illisible).
pub const MAX_GRID_DIRS: usize = 16;

/// Peut-on afficher toutes les directions côte à côte ?
pub fn can_show_all_dirs(pic: &Picture) -> bool {
    (2..=MAX_GRID_DIRS).contains(&pic.directions)
}

/// Disposition des directions : une cellule par direction, toutes de la même taille, alignées sur l'origine commune.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirGrid {
    pub n: usize,
    pub cols: usize,
    pub rows: usize,
    pub cell_w: u32,
    pub cell_h: u32,
    /// Coin haut-gauche de la boîte englobante de toutes les directions (coordonnées de l'origine commune).
    pub x0: i32,
    pub y0: i32,
}

/// Disposition pour l'image : deux rangées dès que l'on dépasse trois directions (3 × 2 pour un FRM de six directions).
pub fn dir_grid(pic: &Picture) -> DirGrid {
    let n = pic.directions.max(1);
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for d in 0..n {
        let b = pic.dir_bounds(d);
        x0 = x0.min(b.0);
        y0 = y0.min(b.1);
        x1 = x1.max(b.2);
        y1 = y1.max(b.3);
    }
    let rows = if n <= 3 { 1 } else { 2 };
    DirGrid { n, cols: n.div_ceil(rows), rows, cell_w: (x1 - x0).max(1) as u32, cell_h: (y1 - y0).max(1) as u32, x0, y0 }
}

impl DirGrid {
    /// Taille de l'ensemble.
    pub fn size(&self) -> (u32, u32) {
        (self.cols as u32 * self.cell_w, self.rows as u32 * self.cell_h)
    }

    /// Coin haut-gauche de la cellule de la direction `d`.
    pub fn cell_origin(&self, d: usize) -> (u32, u32) {
        ((d % self.cols) as u32 * self.cell_w, (d / self.cols) as u32 * self.cell_h)
    }

    /// Position de l'origine commune (le point d'ancrage) dans la cellule de la direction `d`.
    pub fn anchor(&self, d: usize) -> (i32, i32) {
        let (cx, cy) = self.cell_origin(d);
        (cx as i32 - self.x0, cy as i32 - self.y0)
    }

    /// Direction dont la cellule contient le pixel (`x`, `y`) de l'ensemble, avec sa position dans la cellule.
    pub fn cell_at(&self, x: i32, y: i32) -> Option<(usize, i32, i32)> {
        let (w, h) = self.size();
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return None;
        }
        let (col, row) = (x as usize / self.cell_w as usize, y as usize / self.cell_h as usize);
        let d = row * self.cols + col;
        (d < self.n).then(|| (d, x - (col as u32 * self.cell_w) as i32, y - (row as u32 * self.cell_h) as i32))
    }
}

/// Pixels RGBA de toutes les directions côte à côte pour l'image `f` (les directions plus courtes restent sur leur dernière
/// image) : (largeur, hauteur, pixels). Les images sont placées d'après leur position, donc alignées sur l'origine commune.
pub fn render_all_dirs(pic: &Picture, f: usize) -> (usize, usize, Vec<u8>) {
    let g = dir_grid(pic);
    let (w, h) = g.size();
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    for d in 0..g.n {
        let frame = &pic.frames[pic.frame_index(d, f.min(pic.dir_len(d).saturating_sub(1)))];
        let rgba = frame_to_rgba_cow(frame, pic.palette.as_ref());
        let (ax, ay) = g.anchor(d);
        let (ox, oy) = (ax + frame.pos.0, ay + frame.pos.1);
        let (cx, cy) = g.cell_origin(d);
        for y in 0..frame.height as i32 {
            let ty = oy + y;
            // Le dessin reste dans sa cellule.
            if ty < cy as i32 || ty >= (cy + g.cell_h) as i32 {
                continue;
            }
            let x_from = (cx as i32 - ox).max(0);
            let x_to = ((cx + g.cell_w) as i32 - ox).min(frame.width as i32);
            if x_to <= x_from {
                continue;
            }
            let n = (x_to - x_from) as usize * 4;
            let src = (y as usize * frame.width as usize + x_from as usize) * 4;
            let dst = (ty as usize * w + (ox + x_from) as usize) * 4;
            out[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
        }
    }
    (w, h, out)
}

/// Point d'ancrage (le centre de l'hexagone pour un FRM) dans le canevas de la direction : l'origine commune des
/// positions des images.
pub fn anchor_in_canvas(pic: &Picture, dir: usize) -> (i32, i32) {
    let (x0, y0, _, _) = pic.dir_bounds(dir);
    (-x0, -y0)
}

// ----------------------------------------------------------------------------- rotation et symétries

/// Rotation (quarts de tour dans le sens horaire) et symétries appliquées à l'affichage ; l'image n'est jamais modifiée.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Transform {
    pub quarter_turns: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        self.quarter_turns.is_multiple_of(4) && !self.flip_h && !self.flip_v
    }

    /// Valeur 0..15 qui identifie la transformation (clé de cache des textures).
    pub fn id(&self) -> u8 {
        (self.quarter_turns % 4) | ((self.flip_h as u8) << 2) | ((self.flip_v as u8) << 3)
    }

    pub fn rotate_right(&mut self) {
        self.quarter_turns = (self.quarter_turns + 1) % 4;
    }

    pub fn rotate_left(&mut self) {
        self.quarter_turns = (self.quarter_turns + 3) % 4;
    }

    /// Dimensions après transformation d'une image `w` × `h`.
    pub fn size(&self, w: usize, h: usize) -> (usize, usize) {
        if self.quarter_turns % 2 == 1 { (h, w) } else { (w, h) }
    }

    /// Coordonnées dans l'image d'origine (`w` × `h`) du pixel (`tx`, `ty`) de l'image transformée.
    pub fn source_of(&self, tx: usize, ty: usize, w: usize, h: usize) -> (usize, usize) {
        // Avant la rotation, l'image est l'originale retournée.
        let (fx, fy) = match self.quarter_turns % 4 {
            0 => (tx, ty),
            1 => (ty, h - 1 - tx),
            2 => (w - 1 - tx, h - 1 - ty),
            _ => (w - 1 - ty, tx),
        };
        (if self.flip_h { w - 1 - fx } else { fx }, if self.flip_v { h - 1 - fy } else { fy })
    }

    /// Position (en pixels, coin haut-gauche = 0) dans l'image transformée du point (`px`, `py`) de l'image d'origine
    /// `w` × `h` : sert à placer sur la vue un repère exprimé dans l'image (l'ancrage d'un FRM).
    pub fn map_point(&self, px: f32, py: f32, w: usize, h: usize) -> (f32, f32) {
        let (w, h) = (w as f32, h as f32);
        let fx = if self.flip_h { w - px } else { px };
        let fy = if self.flip_v { h - py } else { py };
        match self.quarter_turns % 4 {
            0 => (fx, fy),
            1 => (h - fy, fx),
            2 => (w - fx, h - fy),
            _ => (fy, w - fx),
        }
    }

    /// Pixels RGBA transformés : (pixels, largeur, hauteur).
    pub fn apply(&self, rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
        let (out, tw, th) = self.apply_cow(Cow::Borrowed(rgba), w, h);
        (out.into_owned(), tw, th)
    }

    /// Comme `apply`, mais l'image est rendue telle quelle (sans copie) quand la transformation est l'identité ; sinon les
    /// lignes de l'image transformée sont calculées en parallèle.
    pub fn apply_cow<'a>(&self, rgba: Cow<'a, [u8]>, w: usize, h: usize) -> (Cow<'a, [u8]>, usize, usize) {
        if self.is_identity() {
            return (rgba, w, h);
        }
        let (tw, th) = self.size(w, h);
        let mut out = vec![0u8; tw * th * 4];
        out.par_chunks_mut((tw * 4).max(4)).enumerate().for_each(|(ty, row)| {
            for tx in 0..tw {
                let (sx, sy) = self.source_of(tx, ty, w, h);
                let s = (sy * w + sx) * 4;
                row[tx * 4..tx * 4 + 4].copy_from_slice(&rgba[s..s + 4]);
            }
        });
        (Cow::Owned(out), tw, th)
    }
}

/// Réduit l'image d'un facteur `k` (moyenne des blocs `k` × `k`, pondérée par l'alpha) : (pixels, largeur, hauteur).
pub fn box_downscale(rgba: &[u8], w: usize, h: usize, k: usize) -> (Vec<u8>, usize, usize) {
    let (nw, nh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = vec![0u8; nw * nh * 4];
    out.par_chunks_mut((nw * 4).max(4)).enumerate().for_each(|(y, row)| {
        for x in 0..nw {
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for yy in y * k..((y + 1) * k).min(h) {
                for xx in x * k..((x + 1) * k).min(w) {
                    let p = &rgba[(yy * w + xx) * 4..(yy * w + xx) * 4 + 4];
                    let al = p[3] as u64;
                    r += p[0] as u64 * al;
                    g += p[1] as u64 * al;
                    b += p[2] as u64 * al;
                    a += al;
                    n += 1;
                }
            }
            let o = x * 4;
            if a > 0 {
                row[o] = (r / a) as u8;
                row[o + 1] = (g / a) as u8;
                row[o + 2] = (b / a) as u8;
            }
            row[o + 3] = (a / n.max(1)) as u8;
        }
    });
    (out, nw, nh)
}

// ----------------------------------------------------------------------------- lecture d'un pixel

/// Ce qu'on sait d'un pixel de l'image affichée.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PixelInfo {
    pub x: i32,
    pub y: i32,
    pub rgba: [u8; 4],
    /// Indice dans la palette (images indexées, à l'intérieur de l'image).
    pub index: Option<u8>,
}

/// Pixel (`x`, `y`) du canevas de la direction `dir` pour l'image `frame` ; `None` hors du canevas. Hors de l'image mais
/// dans le canevas (marge d'une animation), le pixel est transparent et n'a pas d'indice.
pub fn pixel_info(pic: &Picture, dir: usize, frame: usize, x: i32, y: i32) -> Option<PixelInfo> {
    let (cw, ch) = canvas_size(pic, dir);
    if x < 0 || y < 0 || x >= cw as i32 || y >= ch as i32 {
        return None;
    }
    let f = &pic.frames[pic.frame_index(dir, frame)];
    let (ox, oy) = canvas_offset(pic, dir, f);
    let (lx, ly) = (x - ox, y - oy);
    if lx < 0 || ly < 0 || lx >= f.width as i32 || ly >= f.height as i32 {
        return Some(PixelInfo { x, y, rgba: [0; 4], index: None });
    }
    let at = ly as usize * f.width as usize + lx as usize;
    Some(match &f.pixels {
        Pixels::Rgba(v) => PixelInfo { x, y, rgba: v[at * 4..at * 4 + 4].try_into().unwrap(), index: None },
        Pixels::Indexed(idx) => {
            let i = idx[at];
            let rgba = match &pic.palette {
                Some(p) if p.used[i as usize] => p.colors[i as usize],
                Some(_) => [0, 0, 0, 255],
                None => [i, i, i, 255],
            };
            PixelInfo { x, y, rgba, index: Some(i) }
        }
    })
}

/// « #RRGGBB » (ou « #RRGGBBAA » si l'alpha n'est pas opaque).
pub fn hex_color(c: [u8; 4]) -> String {
    if c[3] == 255 { format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]) } else { format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3]) }
}

/// Fond affiché derrière l'image.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Background {
    #[default]
    Checker,
    Black,
    White,
    Gray,
    /// Couleur choisie avec le sélecteur de couleur (réglage `bg_custom`).
    Custom,
}

impl Background {
    /// Couleur unie, ou `None` pour le damier ; `custom` est la couleur du fond « personnalisé ».
    pub fn color(self, custom: [u8; 3]) -> Option<[u8; 3]> {
        match self {
            Background::Checker => None,
            Background::Black => Some([0, 0, 0]),
            Background::White => Some([255, 255, 255]),
            Background::Gray => Some([128, 128, 128]),
            Background::Custom => Some(custom),
        }
    }

    /// Fond suivant (touche B).
    pub fn next(self) -> Background {
        match self {
            Background::Checker => Background::Black,
            Background::Black => Background::White,
            Background::White => Background::Gray,
            Background::Gray => Background::Custom,
            Background::Custom => Background::Checker,
        }
    }
}

// ----------------------------------------------------------------------------- couleurs (sélecteur du fond personnalisé)

/// Teinte, saturation et valeur (toutes entre 0 et 1) d'une couleur RVB.
pub fn rgb_to_hsv(c: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// Couleur RVB d'une teinte, saturation et valeur (toutes entre 0 et 1).
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let (c, i) = (v * s, h.floor());
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match i as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

/// « #RRGGBB » en majuscules.
pub fn hex_rgb(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Lit « #RRGGBB », « RRGGBB », « #RGB » ou « RGB » (majuscules ou minuscules).
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().trim_start_matches('#');
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&t[i..=i], 16).ok();
    match t.len() {
        6 => Some([u8::from_str_radix(&t[0..2], 16).ok()?, u8::from_str_radix(&t[2..4], 16).ok()?, u8::from_str_radix(&t[4..6], 16).ok()?]),
        3 => Some([nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17]),
        _ => None,
    }
}

/// Ajoute `c` en tête des couleurs récentes (sans doublon, `max` couleurs au plus).
pub fn push_recent(list: &mut Vec<[u8; 3]>, c: [u8; 3], max: usize) {
    list.retain(|x| *x != c);
    list.insert(0, c);
    list.truncate(max);
}
