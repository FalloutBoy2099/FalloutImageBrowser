//! Conversion d'une `Picture` vers l'image prête à encoder (`Prepared`) puis vers un fichier.

use crate::formats::{self, Caps, Depth, EncodeOpts, OutData, OutFrame, Prepared};
use crate::model::*;
use crate::palette::{self, Mapper};
use anyhow::{Result, bail};
use std::sync::Arc;

#[derive(Clone)]
pub enum PalChoice {
    /// Palette propre à l'image (conservée ou calculée).
    Adaptive,
    /// Palette Fallout 2 (default.pal).
    Fallout,
    /// Palette personnalisée chargée depuis un .PAL.
    Custom(Arc<Palette>),
    /// Palette calculée à partir de l'image (index 0 transparent, couleurs sur 6 bits), à enregistrer en .PAL.
    Create,
}

#[derive(Clone)]
pub enum DepthChoice {
    Indexed(PalChoice),
    Rgb16,
    Rgb24,
    Rgba32,
}

impl DepthChoice {
    pub fn depth(&self) -> Depth {
        match self {
            DepthChoice::Indexed(_) => Depth::Indexed,
            DepthChoice::Rgb16 => Depth::Rgb16,
            DepthChoice::Rgb24 => Depth::Rgb24,
            DepthChoice::Rgba32 => Depth::Rgba32,
        }
    }
}

#[derive(Clone)]
pub struct ConvertOptions {
    pub format: Format,
    pub depth: DepthChoice,
    pub dither: bool,
    pub bg: [u8; 3],
    pub enc: EncodeOpts,
}

/// Image / direction à exporter pour les formats à image unique.
#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub dir: usize,
    pub frame: usize,
}

pub fn depth_supported(c: &Caps, d: &DepthChoice) -> bool {
    match d {
        DepthChoice::Indexed(PalChoice::Adaptive) => c.indexed && !c.no_adaptive,
        DepthChoice::Indexed(PalChoice::Fallout) => c.indexed,
        DepthChoice::Indexed(PalChoice::Custom(_) | PalChoice::Create) => c.indexed && c.custom_pal,
        DepthChoice::Rgb16 => c.d16,
        DepthChoice::Rgb24 => c.d24,
        DepthChoice::Rgba32 => c.d32,
    }
}

fn alpha_kept(format: Format, depth: Depth) -> bool {
    match depth {
        Depth::Rgba32 => format != Format::Jpeg,
        Depth::Rgb16 => matches!(format, Format::Tga | Format::Dds),
        _ => false,
    }
}

fn composite(rgba: &mut [u8], bg: [u8; 3]) {
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a < 255 {
            for c in 0..3 {
                p[c] = ((p[c] as u32 * a + bg[c] as u32 * (255 - a) + 127) / 255) as u8;
            }
            p[3] = 255;
        }
    }
}

fn to_555(rgba: &mut [u8], keep_alpha: bool) {
    for p in rgba.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = (p[c] & 0xF8) | (p[c] >> 5);
        }
        p[3] = if keep_alpha { if p[3] >= 128 { 255 } else { 0 } } else { 255 };
    }
}

pub fn prepare(pic: &Picture, opt: &ConvertOptions, sel: &Selection) -> Result<Prepared> {
    let caps = formats::caps(opt.format);
    if !depth_supported(&caps, &opt.depth) {
        bail!("profondeur non prise en charge par {}", opt.format.name());
    }
    if pic.frames.is_empty() {
        bail!("image vide");
    }
    // --- Sélection des images --------------------------------------------------------------
    let sdir = sel.dir.min(pic.directions.saturating_sub(1));
    let (src, dirs, fpd, dir_shift): (Vec<&Frame>, usize, usize, Vec<(i32, i32)>) = if caps.multi_frame {
        let keep_dirs = match opt.format {
            Format::Frm => pic.directions.min(6),
            Format::Art => pic.directions.min(8),
            _ => 1,
        };
        if keep_dirs == 1 && pic.directions > 1 {
            (pic.dir_frames(sdir).iter().collect(), 1, pic.frames_per_dir, vec![(0, 0)])
        } else {
            let n = keep_dirs * pic.frames_per_dir;
            (pic.frames.iter().take(n).collect(), keep_dirs, pic.frames_per_dir, (0..keep_dirs).map(|d| pic.dir_shift.get(d).copied().unwrap_or((0, 0))).collect())
        }
    } else {
        let i = pic.frame_index(sdir, sel.frame);
        (vec![&pic.frames[i]], 1, 1, vec![(0, 0)])
    };
    if src.is_empty() {
        bail!("aucune image");
    }
    let default_delay = 1000 / pic.fps.max(1);
    let mut out = Prepared {
        format: opt.format,
        depth: opt.depth.depth(),
        frames: Vec::with_capacity(src.len()),
        directions: dirs,
        frames_per_dir: fpd,
        fps: if pic.fps == 0 { 10 } else { pic.fps },
        action_frame: pic.action_frame,
        dir_shift,
        palette: None,
    };
    let meta = |f: &Frame, data: OutData| OutFrame { width: f.width, height: f.height, data, pos: f.pos, delay_ms: Some(f.delay_ms.unwrap_or(default_delay)) };

    match &opt.depth {
        DepthChoice::Rgb16 | DepthChoice::Rgb24 | DepthChoice::Rgba32 => {
            let keep = alpha_kept(opt.format, out.depth);
            for f in &src {
                let mut rgba = frame_to_rgba(f, pic.palette.as_ref());
                if !keep {
                    composite(&mut rgba, opt.bg);
                }
                if out.depth == Depth::Rgb16 {
                    to_555(&mut rgba, keep);
                }
                out.frames.push(meta(f, OutData::Rgba(rgba)));
            }
        }
        DepthChoice::Indexed(choice) => {
            let src_indexed = src.iter().all(|f| matches!(f.pixels, Pixels::Indexed(_))) && pic.palette.is_some();
            let keep_transp = caps.indexed_alpha;
            let fixed = matches!(choice, PalChoice::Fallout | PalChoice::Custom(_) | PalChoice::Create);
            // (palette cible, index transparent, palette alignée sur 4, indices conservés tels quels)
            let (target, transparent, grid, identity): (Palette, Option<u8>, bool, bool) = match choice {
                PalChoice::Fallout => ((*palette::fallout()).clone(), Some(0), true, false),
                PalChoice::Custom(p) => ((**p).clone(), Some(0), true, false),
                PalChoice::Create => {
                    let rgbas: Vec<Vec<u8>> = src.iter().map(|f| frame_to_rgba(f, pic.palette.as_ref())).collect();
                    let refs: Vec<&[u8]> = rgbas.iter().map(|v| &v[..]).collect();
                    let mut p = palette::adaptive_palette(&refs, true, opt.bg);
                    palette::snap_6bit(&mut p);
                    (p, Some(0), true, false)
                }
                PalChoice::Adaptive => {
                    let sp = pic.palette.as_ref();
                    let identity = src_indexed
                        && match sp {
                            Some(sp) => !caps.index0_transparent || sp.colors[0][3] == 0,
                            None => false,
                        };
                    if identity {
                        let sp = sp.unwrap().clone();
                        let t = (0..256usize).find(|&i| sp.used[i] && sp.colors[i][3] == 0).map(|i| i as u8);
                        (sp, t, false, true)
                    } else {
                        let rgbas: Vec<Vec<u8>> = src.iter().map(|f| frame_to_rgba(f, pic.palette.as_ref())).collect();
                        let any_alpha = rgbas.iter().any(|v| v.chunks_exact(4).any(|p| p[3] < 128));
                        let reserve = caps.indexed_alpha && (any_alpha || caps.index0_transparent);
                        let flat: Vec<Vec<u8>>;
                        let refs: Vec<&[u8]> = if !caps.indexed_alpha && any_alpha {
                            flat = rgbas
                                .iter()
                                .map(|v| {
                                    let mut c = v.clone();
                                    composite(&mut c, opt.bg);
                                    c
                                })
                                .collect();
                            flat.iter().map(|v| &v[..]).collect()
                        } else {
                            rgbas.iter().map(|v| &v[..]).collect()
                        };
                        let p = palette::adaptive_palette(&refs, reserve, opt.bg);
                        (p, if reserve { Some(0) } else { None }, false, false)
                    }
                }
            };
            // Palette écrite : sans gestion de la transparence, l'entrée transparente prend la couleur de fond.
            let mut work = target;
            if !keep_transp {
                for i in 0..256 {
                    if work.colors[i][3] < 128 || (fixed && i == 0) {
                        work.colors[i] = [opt.bg[0], opt.bg[1], opt.bg[2], 255];
                        work.used[i] = true;
                    }
                    work.colors[i][3] = 255;
                }
            }
            if identity {
                for f in &src {
                    let Pixels::Indexed(idx) = &f.pixels else { bail!("image indexée attendue") };
                    out.frames.push(meta(f, OutData::Indexed(idx.clone())));
                }
            } else {
                let reserved = if fixed {
                    Some(0)
                } else if keep_transp {
                    transparent
                } else {
                    None
                };
                let mapper = Mapper::new(&work, reserved, grid)?;
                let remap = if src_indexed {
                    let src_pal = pic.palette.as_ref().unwrap();
                    let sp = if keep_transp { src_pal.clone() } else { palette::flatten_palette(src_pal, opt.bg) };
                    let mut t = palette::remap_table(&sp, &work, &mapper);
                    // Les pixels transparents restent sur l'index 0 (fond ou transparence).
                    if fixed {
                        for i in 0..256 {
                            if src_pal.colors[i][3] == 0 {
                                t[i] = 0;
                            }
                        }
                    }
                    Some(t)
                } else {
                    None
                };
                for f in &src {
                    let data = match (&f.pixels, &remap) {
                        (Pixels::Indexed(idx), Some(t)) => OutData::Indexed(idx.iter().map(|&i| t[i as usize]).collect()),
                        _ => {
                            let mut rgba = frame_to_rgba(f, pic.palette.as_ref());
                            if !keep_transp {
                                composite(&mut rgba, opt.bg);
                            }
                            OutData::Indexed(palette::quantize_rgba(&rgba, f.width as usize, f.height as usize, &work, &mapper, opt.dither))
                        }
                    };
                    out.frames.push(meta(f, data));
                }
            }
            out.palette = Some(work);
        }
    }
    Ok(out)
}

pub fn convert_to_bytes(pic: &Picture, opt: &ConvertOptions, sel: &Selection) -> Result<Vec<u8>> {
    let prep = prepare(pic, opt, sel)?;
    formats::encode(&prep, &opt.enc)
}

/// Extrait une image unique d'une `Picture` (pour l'export en séquence d'images).
pub fn single_frame(pic: &Picture, dir: usize, frame: usize) -> Picture {
    let i = pic.frame_index(dir, frame);
    let mut p = pic.clone();
    p.frames = vec![pic.frames[i].clone()];
    p.directions = 1;
    p.frames_per_dir = 1;
    p.dir_shift = vec![(0, 0)];
    p.sections.clear();
    p
}
