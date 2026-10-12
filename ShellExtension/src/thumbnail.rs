//! Rendu d'une vignette à partir d'un fichier Fallout (FRM, RIX), Arcanum (ART) ou Fallout Tactics (ZAR, TIL, SPR) (partie sans dépendance à Windows, donc testable partout).

use fallout_image_browser::formats::{self, DecodeCtx};
use fallout_image_browser::model::Palette;
use fallout_image_browser::model::{Format, Picture, frame_to_rgba};
use fallout_image_browser::palette;
use std::path::Path;
use std::sync::Arc;

/// Taille maximale lue dans le flux : un fichier plus gros n'est pas une image Fallout.
pub const MAX_INPUT: usize = 64 << 20;

/// Vignette prête pour un bitmap Windows : pixels BGRA 8 bits, couleurs prémultipliées par l'alpha, lignes de haut en bas.
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Format d'après le contenu : signature connue, sinon FRM (qui n'a pas de signature).
fn detect(data: &[u8]) -> Format {
    if data.starts_with(b"RIX") {
        return Format::Rix;
    }
    formats::sniff(data).unwrap_or(Format::Frm)
}

/// Décode le contenu d'un flux (palette Fallout par défaut, première image de la première direction) et le réduit pour que
/// sa plus grande dimension ne dépasse pas `max_size`. Une image plus petite n'est pas agrandie.
pub fn render(data: &[u8], max_size: u32) -> Result<Thumb, String> {
    if data.len() > MAX_INPUT {
        return Err("fichier trop volumineux".into());
    }
    let ctx = DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() };
    let pic = formats::decode(detect(data), data, &ctx).map_err(|e| e.to_string())?;
    render_picture(&pic, max_size)
}

/// Palette d'un `.PAL` du même nom que le fichier (dans le même dossier), s'il existe et se lit.
fn sibling_palette(path: &Path) -> Option<Arc<Palette>> {
    ["pal", "PAL", "Pal"].iter().find_map(|ext| {
        let p = path.with_extension(ext);
        let len = std::fs::metadata(&p).ok()?.len();
        (p.is_file() && len < 1 << 20).then_some(())?;
        palette::parse_pal(&std::fs::read(&p).ok()?).ok().map(Arc::new)
    })
}

/// Vignette d'un contenu accompagné du nom que le flux déclare (nom seul, chemin complet ou rien) : l'extension choisit le
/// format (ART n'a pas de signature), et si le nom est un chemin complet un `.PAL` du même nom remplace la palette Fallout,
/// comme dans l'application. Un `.PAL` illisible (accès refusé par l'isolation…) est simplement ignoré.
pub fn render_named(data: &[u8], name: Option<&str>, max_size: u32) -> Result<Thumb, String> {
    if data.len() > MAX_INPUT {
        return Err("fichier trop volumineux".into());
    }
    let path = name.map(Path::new);
    let ext = path.and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let format = Format::from_ext(&ext).unwrap_or_else(|| detect(data));
    let palette = path.filter(|p| p.is_absolute()).and_then(sibling_palette).unwrap_or_else(palette::fallout);
    let ctx = DecodeCtx { palette, palette_name: "Fallout".into(), ext };
    let pic = formats::decode(format, data, &ctx).map_err(|e| e.to_string())?;
    render_picture(&pic, max_size)
}

/// Vignette d'un fichier du disque (mêmes règles que `render_named`).
pub fn render_file(path: &Path, max_size: u32) -> Result<Thumb, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_INPUT as u64 {
        return Err("fichier trop volumineux".into());
    }
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    render_named(&data, Some(&path.to_string_lossy()), max_size)
}

/// Première image de la première direction, réduite à `max_size`.
pub fn render_picture(pic: &Picture, max_size: u32) -> Result<Thumb, String> {
    let frame = pic.frames.first().ok_or("aucune image")?;
    let rgba = frame_to_rgba(frame, pic.palette.as_ref());
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Err("image vide".into());
    }
    Ok(scale_premultiplied(&rgba, w, h, max_size.max(1) as usize))
}

/// Réduction par moyenne de boîte, calculée sur des couleurs prémultipliées (les bords transparents ne bavent pas).
fn scale_premultiplied(rgba: &[u8], w: usize, h: usize, max_size: usize) -> Thumb {
    let (nw, nh) = if w.max(h) <= max_size {
        (w, h)
    } else if w >= h {
        (max_size, (h * max_size / w).max(1))
    } else {
        ((w * max_size / h).max(1), max_size)
    };
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1).min(h));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1).min(w));
            let (mut b, mut g, mut r, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &rgba[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
                    let al = p[3] as u32;
                    r += p[0] as u32 * al / 255;
                    g += p[1] as u32 * al / 255;
                    b += p[2] as u32 * al / 255;
                    a += al;
                    n += 1;
                }
            }
            let o = (y * nw + x) * 4;
            out[o] = (b / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (r / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    Thumb { width: nw as u32, height: nh as u32, bgra: out }
}
