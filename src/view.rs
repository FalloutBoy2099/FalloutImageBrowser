//! Calculs d'affichage sans interface (donc testables) : canevas d'une direction, rotation / symétries de la vue,
//! lecture d'un pixel.

use crate::model::*;

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
    let frame = &pic.frames[pic.frame_index(dir, f)];
    let (cw, ch) = canvas_size(pic, dir);
    let rgba = frame_to_rgba(frame, pic.palette.as_ref());
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
    (cw, ch, canvas)
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
        if self.is_identity() {
            return (rgba.to_vec(), w, h);
        }
        let (tw, th) = self.size(w, h);
        let mut out = vec![0u8; tw * th * 4];
        for ty in 0..th {
            for tx in 0..tw {
                let (sx, sy) = self.source_of(tx, ty, w, h);
                let (s, d) = ((sy * w + sx) * 4, (ty * tw + tx) * 4);
                out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
            }
        }
        (out, tw, th)
    }
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
