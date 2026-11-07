//! Assemblage d'un FRM (animé, 1 à 6 directions) à partir d'une suite d'images : PNG, GIF animé, autre FRM, etc.
//! Les images sont réduites à la palette choisie (Fallout par défaut) ; chaque image est centrée sur son point d'ancrage
//! (milieu du bas), comme le fait la conversion ordinaire vers FRM.

use crate::convert::{ConvertOptions, DepthChoice, PalChoice, Selection, convert_to_bytes};
use crate::formats::EncodeOpts;
use crate::imageops::ImageOps;
use crate::loader::{PalPolicy, load_file};
use crate::model::*;
use anyhow::{Result, bail};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AssembleOptions {
    /// Nombre de directions (1 à 6) : les images sont réparties en parts égales, dans l'ordre.
    pub directions: usize,
    pub fps: u32,
    pub action_frame: u32,
    /// Palette cible ; `None` = palette Fallout.
    pub palette: Option<Arc<Palette>>,
    pub dither: bool,
}

impl Default for AssembleOptions {
    fn default() -> Self {
        AssembleOptions { directions: 1, fps: 10, action_frame: 0, palette: None, dither: false }
    }
}

/// Toutes les images d'une liste de fichiers (un GIF ou un FRM en fournit plusieurs), en RVBA.
pub fn load_frames(files: &[PathBuf]) -> Result<Vec<Frame>> {
    let mut out = Vec::new();
    for path in files {
        let loaded = load_file(path, &PalPolicy::Fallout).map_err(|e| anyhow::anyhow!("{} : {e}", path.display()))?;
        let pic = &loaded.pic;
        if pic.frames.is_empty() {
            bail!("{} : aucune image", path.display());
        }
        for f in &pic.frames {
            let rgba = frame_to_rgba(f, pic.palette.as_ref());
            let (w, h) = (f.width as i32, f.height as i32);
            out.push(Frame { width: f.width, height: f.height, pixels: Pixels::Rgba(rgba), pos: (-(w / 2), -h), delay_ms: None });
        }
    }
    Ok(out)
}

/// Image multi-directions prête à être convertie.
pub fn build_picture(frames: Vec<Frame>, opt: &AssembleOptions) -> Result<Picture> {
    let dirs = opt.directions;
    if !(1..=6).contains(&dirs) {
        bail!("le nombre de directions doit être compris entre 1 et 6");
    }
    if frames.is_empty() {
        bail!("aucune image à assembler");
    }
    if !frames.len().is_multiple_of(dirs) {
        bail!("{} images ne se répartissent pas en {dirs} directions de même longueur", frames.len());
    }
    let fpd = frames.len() / dirs;
    if fpd > u16::MAX as usize {
        bail!("trop d'images par direction");
    }
    let mut pic = Picture::new(Format::Frm);
    pic.directions = dirs;
    pic.frames_per_dir = fpd;
    pic.fps = opt.fps.clamp(1, 65535);
    pic.action_frame = opt.action_frame.min(fpd as u32 - 1);
    pic.dir_shift = vec![(0, 0); dirs];
    pic.frames = frames;
    Ok(pic)
}

/// Octets du FRM obtenu.
pub fn assemble_frm(frames: Vec<Frame>, opt: &AssembleOptions) -> Result<Vec<u8>> {
    let pic = build_picture(frames, opt)?;
    picture_to_frm(&pic, opt.palette.clone(), opt.dither)
}

/// Encode une image (indexée ou non) en FRM avec la palette donnée (Fallout par défaut).
pub fn picture_to_frm(pic: &Picture, palette: Option<Arc<Palette>>, dither: bool) -> Result<Vec<u8>> {
    let choice = match palette {
        Some(p) => PalChoice::Custom(p),
        None => PalChoice::Fallout,
    };
    let conv =
        ConvertOptions { format: Format::Frm, depth: DepthChoice::Indexed(choice), dither, bg: [0, 0, 0], enc: EncodeOpts::default(), ops: ImageOps::default(), text_white: false };
    convert_to_bytes(pic, &conv, &Selection::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, rgba: [u8; 4]) -> Frame {
        let px: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        Frame { width: w, height: h, pixels: Pixels::Rgba(px), pos: (-(w as i32 / 2), -(h as i32)), delay_ms: None }
    }

    #[test]
    fn directions_must_divide_the_frames() {
        let frames = vec![frame(2, 2, [255, 0, 0, 255]); 5];
        assert!(build_picture(frames, &AssembleOptions { directions: 2, ..Default::default() }).is_err());
    }

    #[test]
    fn builds_a_two_direction_frm_that_reads_back() {
        let frames = vec![frame(4, 4, [252, 0, 0, 255]), frame(4, 4, [0, 0, 252, 255]), frame(6, 3, [0, 252, 0, 255]), frame(4, 4, [252, 252, 0, 255])];
        let opt = AssembleOptions { directions: 2, fps: 8, action_frame: 1, ..Default::default() };
        let bytes = assemble_frm(frames, &opt).unwrap();
        let h = crate::formats::frm::read_header(&bytes).unwrap();
        assert_eq!((h.directions, h.frames_per_direction, h.fps, h.action_frame), (2, 2, 8, 1));
    }

    #[test]
    fn action_frame_is_clamped() {
        let frames = vec![frame(2, 2, [252, 0, 0, 255]); 3];
        let pic = build_picture(frames, &AssembleOptions { action_frame: 99, ..Default::default() }).unwrap();
        assert_eq!(pic.action_frame, 2);
    }
}
