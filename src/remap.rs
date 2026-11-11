//! Remappage d'une image indexée vers une autre palette : chaque pixel garde sa place et reçoit l'index de la couleur la plus
//! proche dans la palette cible (l'index 0 reste transparent).

use crate::model::*;
use crate::palette::{self, Mapper};
use anyhow::{Result, bail};

/// Image équivalente utilisant la palette `target`. Échoue si l'image n'est pas indexée.
pub fn remap_picture(pic: &Picture, target: &Palette) -> Result<Picture> {
    let Some(src) = &pic.palette else { bail!("l'image n'a pas de palette") };
    if pic.frames.iter().any(|f| !matches!(f.pixels, Pixels::Indexed(_))) {
        bail!("l'image n'est pas indexée");
    }
    let mapper = Mapper::new(target, Some(0), palette::is_grid_aligned(target))?;
    let table = palette::remap_table(src, target, &mapper);
    let mut out = pic.clone();
    for f in out.frames.iter_mut() {
        if let Pixels::Indexed(v) = &mut f.pixels {
            for px in v.iter_mut() {
                *px = table[*px as usize];
            }
        }
    }
    out.palette = Some(target.clone());
    out.sections.clear();
    Ok(out)
}

/// Fichier à enregistrer pour une image remappée.
pub struct Remapped {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    /// Palette cible au format `.PAL` Fallout, à écrire à côté d'un FRM : un FRM n'embarque pas sa palette, et ses index n'ont
    /// de sens qu'avec elle. `None` si elle n'est pas nécessaire (palette Fallout, ou format qui garde sa palette).
    pub pal: Option<Vec<u8>>,
}

/// Octets du fichier à enregistrer pour l'image remappée : FRM pour un FRM (palette cible externe), sinon PNG indexé qui
/// conserve la palette cible.
pub fn encode_remapped(result: &Picture) -> Result<Remapped> {
    if result.format == Format::Frm {
        let target = result.palette.clone().map(std::sync::Arc::new);
        let pal = target.as_ref().filter(|t| ***t != *palette::fallout()).map(|t| palette::write_pal(t));
        return Ok(Remapped { bytes: crate::assemble::picture_to_frm(result, target, false)?, ext: "frm", pal });
    }
    let opt = crate::convert::ConvertOptions {
        format: Format::Png,
        depth: crate::convert::DepthChoice::Indexed(crate::convert::PalChoice::Adaptive),
        dither: false,
        bg: [0, 0, 0],
        enc: Default::default(),
        ops: Default::default(),
        text_white: false,
    };
    Ok(Remapped { bytes: crate::convert::convert_to_bytes(result, &opt, &Default::default())?, ext: "png", pal: None })
}

/// Combien de pixels changent d'index avec la nouvelle palette (pour informer avant d'enregistrer).
pub fn changed_pixels(before: &Picture, after: &Picture) -> usize {
    before
        .frames
        .iter()
        .zip(&after.frames)
        .map(|(a, b)| match (&a.pixels, &b.pixels) {
            (Pixels::Indexed(x), Pixels::Indexed(y)) => x.iter().zip(y).filter(|(p, q)| p != q).count(),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(palette: Palette, idx: Vec<u8>) -> Picture {
        let mut p = Picture::new(Format::Frm);
        p.palette = Some(palette);
        p.frames.push(Frame { width: idx.len() as u32, height: 1, pixels: Pixels::Indexed(idx), pos: (0, 0), delay_ms: None });
        p
    }

    #[test]
    fn same_palette_changes_nothing() {
        let pal = (*palette::fallout()).clone();
        let p = pic(pal.clone(), vec![0, 5, 100, 200]);
        let r = remap_picture(&p, &pal).unwrap();
        assert_eq!(changed_pixels(&p, &r), 0);
    }

    #[test]
    fn colours_follow_the_nearest_entry() {
        let mut src = Palette::empty();
        src.colors[0][3] = 0;
        src.used[0] = true;
        src.colors[1] = [252, 0, 0, 255];
        src.used[1] = true;
        let mut dst = Palette::empty();
        dst.colors[0][3] = 0;
        dst.used[0] = true;
        dst.colors[7] = [252, 0, 0, 255];
        dst.used[7] = true;
        dst.colors[8] = [0, 0, 252, 255];
        dst.used[8] = true;
        let p = pic(src, vec![0, 1]);
        let r = remap_picture(&p, &dst).unwrap();
        let Pixels::Indexed(v) = &r.frames[0].pixels else { panic!() };
        assert_eq!(v, &vec![0, 7]);
        assert_eq!(r.palette.as_ref().unwrap(), &dst);
    }

    #[test]
    fn rgba_images_are_refused() {
        let mut p = Picture::new(Format::Png);
        p.frames.push(Frame { width: 1, height: 1, pixels: Pixels::Rgba(vec![0, 0, 0, 255]), pos: (0, 0), delay_ms: None });
        assert!(remap_picture(&p, &palette::fallout()).is_err());
    }
}
