//! Aseprite (.ase / .aseprite) : lecture seule, images d'animation recomposées (calques visibles, mode Normal).
//! https://github.com/aseprite/aseprite/blob/main/docs/ase-file-specs.md

use super::layered::blend_over;
use super::*;
use std::io::Read;

struct LayerInfo {
    visible: bool,
    opacity: f32,
    group: bool,
    background: bool,
}

struct Cel {
    layer: usize,
    x: i32,
    y: i32,
    opacity: f32,
    z: i32,
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    link: Option<usize>,
}

fn string(r: &mut Rd) -> Result<String> {
    let n = r.u16le()? as usize;
    Ok(String::from_utf8_lossy(r.bytes(n)?).to_string())
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    r.skip(4)?;
    if r.u16le()? != 0xA5E0 {
        bail!("signature Aseprite absente");
    }
    let nframes = r.u16le()? as usize;
    let (w, h) = (r.u16le()? as usize, r.u16le()? as usize);
    let depth = r.u16le()? as usize;
    let flags = r.u32le()?;
    let speed = r.u16le()? as u32;
    r.skip(8)?;
    let transparent_index = r.u8()?;
    check_dims(w as u64, h as u64)?;
    if !matches!(depth, 8 | 16 | 32) || nframes == 0 {
        bail!("Aseprite : profondeur ou nombre d'images invalide");
    }
    let mut pos = 128usize;
    let mut layers: Vec<LayerInfo> = Vec::new();
    let mut vis_by_level: Vec<bool> = vec![true; 64];
    let mut palette = [[0u8, 0, 0, 255]; 256];
    let mut frame_cels: Vec<Vec<Cel>> = Vec::with_capacity(nframes);
    let mut durations: Vec<u32> = Vec::with_capacity(nframes);
    for _ in 0..nframes {
        let mut fr = Rd::at(data, pos);
        let fsize = fr.u32le()? as usize;
        if fr.u16le()? != 0xF1FA {
            bail!("Aseprite : en-tête d'image invalide");
        }
        let old_chunks = fr.u16le()? as usize;
        let dur = fr.u16le()? as u32;
        fr.skip(2)?;
        let new_chunks = fr.u32le()? as usize;
        let nchunks = if new_chunks == 0 { old_chunks } else { new_chunks };
        durations.push(if dur == 0 { speed } else { dur });
        let mut cels: Vec<Cel> = Vec::new();
        let mut cp = pos + 16;
        for _ in 0..nchunks {
            let mut c = Rd::at(data, cp);
            let csize = c.u32le()? as usize;
            let ctype = c.u16le()?;
            if csize < 6 {
                bail!("Aseprite : fragment invalide");
            }
            let body = data.get(cp + 6..cp + csize).ok_or_else(|| anyhow!("Aseprite : fichier tronqué"))?;
            let mut b = Rd::new(body);
            match ctype {
                0x2004 => {
                    let lflags = b.u16le()?;
                    let ltype = b.u16le()?;
                    let level = (b.u16le()? as usize).min(63);
                    b.skip(4)?; // taille par défaut
                    b.skip(2)?; // mode de fusion (traité comme Normal)
                    let opacity = b.u8()?;
                    let parent_visible = if level == 0 { true } else { vis_by_level[level - 1] };
                    let visible = lflags & 1 != 0 && parent_visible;
                    vis_by_level[level] = visible;
                    layers.push(LayerInfo { visible, opacity: if flags & 1 != 0 { opacity as f32 / 255.0 } else { 1.0 }, group: ltype == 1, background: lflags & 8 != 0 });
                }
                0x2005 => {
                    let layer = b.u16le()? as usize;
                    let (x, y) = (b.u16le()? as i16 as i32, b.u16le()? as i16 as i32);
                    let opacity = b.u8()? as f32 / 255.0;
                    let ctype2 = b.u16le()?;
                    let z = b.u16le()? as i16 as i32;
                    b.skip(5)?;
                    match ctype2 {
                        1 => {
                            let link = b.u16le()? as usize;
                            cels.push(Cel { layer, x, y, opacity, z, w: 0, h: 0, rgba: Vec::new(), link: Some(link) });
                        }
                        0 | 2 => {
                            let (cw, ch) = (b.u16le()? as usize, b.u16le()? as usize);
                            let bpp = depth / 8;
                            let rest = &body[b.pos..];
                            let raw: Vec<u8> = if ctype2 == 0 {
                                rest.to_vec()
                            } else {
                                let mut v = Vec::with_capacity(cw * ch * bpp);
                                flate2::read::ZlibDecoder::new(rest).read_to_end(&mut v).map_err(|e| anyhow!("Aseprite zlib : {e}"))?;
                                v
                            };
                            if raw.len() < cw * ch * bpp {
                                bail!("Aseprite : pixels de cel insuffisants");
                            }
                            let mut rgba = vec![0u8; cw * ch * 4];
                            for i in 0..cw * ch {
                                let o = i * 4;
                                match depth {
                                    32 => rgba[o..o + 4].copy_from_slice(&raw[i * 4..i * 4 + 4]),
                                    16 => {
                                        rgba[o..o + 3].fill(raw[i * 2]);
                                        rgba[o + 3] = raw[i * 2 + 1];
                                    }
                                    _ => {
                                        let idx = raw[i];
                                        rgba[o..o + 4].copy_from_slice(&palette[idx as usize]);
                                        if idx == transparent_index && !layers.get(layer).is_some_and(|l| l.background) {
                                            rgba[o + 3] = 0;
                                        }
                                    }
                                }
                            }
                            cels.push(Cel { layer, x, y, opacity, z, w: cw, h: ch, rgba, link: None });
                        }
                        _ => {} // cartes de tuiles : ignorées
                    }
                }
                0x2019 => {
                    b.skip(4)?;
                    let (first, last) = (b.u32le()? as usize, b.u32le()? as usize);
                    b.skip(8)?;
                    for i in first..=last.min(255) {
                        let ef = b.u16le()?;
                        let c = b.bytes(4)?;
                        palette[i] = [c[0], c[1], c[2], c[3]];
                        if ef & 1 != 0 {
                            string(&mut b)?;
                        }
                    }
                }
                0x0004 => {
                    // Ancienne palette (par paquets) si aucune nouvelle palette n'est présente.
                    let packets = b.u16le()?;
                    let mut i = 0usize;
                    for _ in 0..packets {
                        i += b.u8()? as usize;
                        let n = match b.u8()? {
                            0 => 256,
                            n => n as usize,
                        };
                        for _ in 0..n {
                            let c = b.bytes(3)?;
                            if i < 256 {
                                palette[i] = [c[0], c[1], c[2], 255];
                            }
                            i += 1;
                        }
                    }
                }
                _ => {}
            }
            cp += csize;
        }
        frame_cels.push(cels);
        pos += fsize;
    }
    // Résolution des cels liés (copie du cel de la même couche dans l'image désignée).
    for f in 0..frame_cels.len() {
        for c in 0..frame_cels[f].len() {
            if let Some(src_frame) = frame_cels[f][c].link {
                let layer = frame_cels[f][c].layer;
                if let Some(s) = frame_cels.get(src_frame).and_then(|v| v.iter().find(|x| x.layer == layer && x.link.is_none())) {
                    let (sw, sh, srgba) = (s.w, s.h, s.rgba.clone());
                    let cel = &mut frame_cels[f][c];
                    cel.w = sw;
                    cel.h = sh;
                    cel.rgba = srgba;
                    cel.link = None;
                }
            }
        }
    }
    check_anim_budget(frame_cels.len(), w * h * 4)?;
    let mut pic = Picture::new(Format::Ase);
    for (f, cels) in frame_cels.iter_mut().enumerate() {
        cels.sort_by_key(|c| (c.layer as i32 + c.z, c.z));
        let mut canvas = vec![0u8; w * h * 4];
        for c in cels.iter() {
            let Some(l) = layers.get(c.layer) else { continue };
            if !l.visible || l.group || c.rgba.is_empty() {
                continue;
            }
            blend_over(&mut canvas, w, h, &c.rgba, c.w, c.h, c.x as i64, c.y as i64, c.opacity * l.opacity);
        }
        pic.frames.push(Frame { width: w as u32, height: h as u32, pixels: Pixels::Rgba(canvas), pos: (0, 0), delay_ms: Some(durations[f].max(10)) });
    }
    let n = pic.frames.len();
    pic.frames_per_dir = n;
    let total: u32 = durations.iter().map(|d| (*d).max(10)).sum();
    pic.fps = ((n as u32 * 1000) / total.max(1)).max(1);
    pic.bpp = depth as u32;
    let s = pic.section("sec.format");
    s.raw(
        "Color mode",
        match depth {
            32 => "RGBA",
            16 => "Grayscale",
            _ => "Indexed",
        },
    );
    s.add("p.frames", n);
    s.add("p.layers", layers.iter().filter(|l| !l.group).count());
    s.add("p.duration", format!("{:.2} s", total as f64 / 1000.0));
    Ok(pic)
}
