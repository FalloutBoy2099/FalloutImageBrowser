//! PNG : palette 8 bits (avec transparence tRNS), 24 et 32 bits ; lecture de tous les types de PNG (APNG : première image).

use super::*;
use ::png as pngc;
use std::io::Cursor;

fn color_name(c: pngc::ColorType) -> &'static str {
    match c {
        pngc::ColorType::Grayscale => "Grayscale",
        pngc::ColorType::Rgb => "RGB",
        pngc::ColorType::Indexed => "Indexed",
        pngc::ColorType::GrayscaleAlpha => "Grayscale + alpha",
        pngc::ColorType::Rgba => "RGBA",
    }
}

fn find(d: &[u8], pat: &[u8]) -> Option<usize> {
    d.windows(pat.len()).position(|w| w == pat)
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    // APNG : un fragment « acTL » précède les données d'image.
    if let (Some(a), Some(i)) = (find(data, b"acTL"), find(data, b"IDAT"))
        && a < i
        && let Ok(p) = decode_apng(data)
    {
        return Ok(p);
    }
    decode_static(data)
}

fn decode_static(data: &[u8]) -> Result<Picture> {
    let mut dec = pngc::Decoder::new(Cursor::new(data));
    let hdr_color = dec.read_header_info().map_err(|e| anyhow!("PNG : {e}"))?.color_type;
    let indexed = hdr_color == pngc::ColorType::Indexed;
    dec.set_transformations(if indexed { pngc::Transformations::IDENTITY } else { pngc::Transformations::normalize_to_color8() });
    let mut reader = dec.read_info().map_err(|e| anyhow!("PNG : {e}"))?;
    let info = reader.info().clone();
    let (w, h) = (info.width as usize, info.height as usize);
    check_dims(w as u64, h as u64)?;
    let size = reader.output_buffer_size().ok_or_else(|| anyhow!("PNG trop grand"))?;
    let mut buf = vec![0u8; size];
    let out = reader.next_frame(&mut buf).map_err(|e| anyhow!("PNG : {e}"))?;
    let line = out.line_size;
    let mut pic;
    if indexed {
        let depth = out.bit_depth as usize;
        let mut idx = vec![0u8; w * h];
        for y in 0..h {
            let row = &buf[y * line..(y + 1) * line];
            for x in 0..w {
                idx[y * w + x] = if depth == 8 {
                    row[x]
                } else {
                    let per = 8 / depth;
                    (row[x / per] >> (8 - depth - (x % per) * depth)) & ((1 << depth) - 1)
                };
            }
        }
        let mut pal = Palette::empty();
        let plte = info.palette.as_deref().unwrap_or(&[]);
        let trns = info.trns.as_deref().unwrap_or(&[]);
        for (i, c) in plte.chunks_exact(3).take(256).enumerate() {
            pal.colors[i] = [c[0], c[1], c[2], trns.get(i).copied().unwrap_or(255)];
            pal.used[i] = true;
        }
        pic = Picture::single(Format::Png, w as u32, h as u32, Pixels::Indexed(idx), Some(pal));
        pic.bpp = depth as u32;
    } else {
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            let row = &buf[y * line..(y + 1) * line];
            for x in 0..w {
                let o = (y * w + x) * 4;
                match out.color_type {
                    pngc::ColorType::Grayscale => rgba[o..o + 3].fill(row[x]),
                    pngc::ColorType::GrayscaleAlpha => {
                        rgba[o..o + 3].fill(row[x * 2]);
                        rgba[o + 3] = row[x * 2 + 1];
                    }
                    pngc::ColorType::Rgb => rgba[o..o + 3].copy_from_slice(&row[x * 3..x * 3 + 3]),
                    pngc::ColorType::Rgba => rgba[o..o + 4].copy_from_slice(&row[x * 4..x * 4 + 4]),
                    pngc::ColorType::Indexed => bail!("PNG : type de couleur inattendu"),
                }
            }
        }
        pic = Picture::single(Format::Png, w as u32, h as u32, Pixels::Rgba(rgba), None);
        pic.bpp = match info.color_type {
            pngc::ColorType::Grayscale => info.bit_depth as u32,
            pngc::ColorType::GrayscaleAlpha => info.bit_depth as u32 * 2,
            pngc::ColorType::Rgb => info.bit_depth as u32 * 3,
            _ => info.bit_depth as u32 * 4,
        };
    }
    let s = pic.section("sec.format");
    s.add("p.compression", "Deflate");
    s.add("p.precision", info.bit_depth as u32);
    s.raw("Color type", color_name(info.color_type));
    s.add("p.interlaced", if info.interlaced { "p.yes" } else { "p.no" });
    if let Some(d) = &info.pixel_dims
        && d.unit == pngc::Unit::Meter
    {
        s.add("p.dpi", format!("{} × {}", dpi_from_ppm(d.xppu), dpi_from_ppm(d.yppu)));
    }
    if let Some(a) = &info.animation_control {
        s.add("p.animated", "p.yes");
        s.add("p.frames", a.num_frames);
    }
    if info.icc_profile.is_some() {
        s.raw("ICC profile", "yes");
    }
    for t in info.uncompressed_latin1_text.iter().take(20) {
        s.raw(&t.keyword, t.text.chars().take(200).collect::<String>());
    }
    Ok(pic)
}

pub fn encode(p: &Prepared, opts: &super::EncodeOpts) -> Result<Vec<u8>> {
    let f = p.first();
    let mut out = Vec::new();
    {
        let mut enc = pngc::Encoder::new(&mut out, f.width, f.height);
        enc.set_depth(pngc::BitDepth::Eight);
        enc.set_compression(match opts.png_level {
            super::PngLevel::Fast => pngc::Compression::Fast,
            super::PngLevel::Normal => pngc::Compression::Balanced,
            super::PngLevel::Best => pngc::Compression::High,
        });
        match (&f.data, &p.palette) {
            (OutData::Indexed(idx), Some(pal)) => {
                enc.set_color(pngc::ColorType::Indexed);
                let n = (0..256).rev().find(|&i| pal.used[i]).map(|i| i + 1).unwrap_or(1);
                let mut plte = Vec::with_capacity(n * 3);
                let mut trns = Vec::with_capacity(n);
                for i in 0..n {
                    let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
                    plte.extend_from_slice(&c[..3]);
                    trns.push(c[3]);
                }
                enc.set_palette(plte);
                // tRNS : seulement jusqu'à la dernière entrée non opaque.
                if let Some(last) = trns.iter().rposition(|&a| a != 255) {
                    trns.truncate(last + 1);
                    enc.set_trns(trns);
                }
                let mut w = enc.write_header().map_err(|e| anyhow!("PNG : {e}"))?;
                w.write_image_data(idx).map_err(|e| anyhow!("PNG : {e}"))?;
            }
            (OutData::Rgba(px), _) => {
                if p.depth == Depth::Rgba32 {
                    enc.set_color(pngc::ColorType::Rgba);
                    let mut w = enc.write_header().map_err(|e| anyhow!("PNG : {e}"))?;
                    w.write_image_data(px).map_err(|e| anyhow!("PNG : {e}"))?;
                } else {
                    enc.set_color(pngc::ColorType::Rgb);
                    let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
                    let mut w = enc.write_header().map_err(|e| anyhow!("PNG : {e}"))?;
                    w.write_image_data(&rgb).map_err(|e| anyhow!("PNG : {e}"))?;
                }
            }
            _ => bail!("PNG : palette manquante"),
        }
    }
    Ok(out)
}

/// APNG : toutes les images sont recomposées sur le canevas (modes de disposition et de fusion pris en compte).
fn decode_apng(data: &[u8]) -> Result<Picture> {
    let mut dec = pngc::Decoder::new(Cursor::new(data));
    dec.set_transformations(pngc::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| anyhow!("PNG : {e}"))?;
    let info = reader.info().clone();
    let (w, h) = (info.width as usize, info.height as usize);
    check_dims(w as u64, h as u64)?;
    let total = info.animation_control.as_ref().map(|a| a.num_frames as usize).ok_or_else(|| anyhow!("pas d'animation"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or_else(|| anyhow!("PNG trop grand"))?];
    let mut canvas = vec![0u8; w * h * 4];
    let mut pic = Picture::new(Format::Png);
    let mut prev: Option<(pngc::FrameControl, Vec<u8>)> = None; // (contrôle, copie avant dessin)
    let mut decoded = 0usize;
    while decoded < total {
        let out = reader.next_frame(&mut buf).map_err(|e| anyhow!("PNG : {e}"))?;
        // Image par défaut qui ne fait pas partie de l'animation : ignorée.
        let Some(fc) = reader.info().frame_control else { continue };
        decoded += 1;
        // Disposition de l'image précédente.
        if let Some((pfc, snapshot)) = prev.take() {
            match pfc.dispose_op {
                pngc::DisposeOp::None => {}
                pngc::DisposeOp::Background => {
                    for y in pfc.y_offset as usize..((pfc.y_offset + pfc.height) as usize).min(h) {
                        let (a, b) = ((y * w + pfc.x_offset as usize) * 4, (y * w + ((pfc.x_offset + pfc.width) as usize).min(w)) * 4);
                        canvas[a..b].fill(0);
                    }
                }
                pngc::DisposeOp::Previous => canvas = snapshot,
            }
        }
        let snapshot = canvas.clone();
        let (fw, fh) = (fc.width as usize, fc.height as usize);
        for y in 0..fh {
            let cy = fc.y_offset as usize + y;
            if cy >= h {
                break;
            }
            for x in 0..fw {
                let cx = fc.x_offset as usize + x;
                if cx >= w {
                    break;
                }
                let s = y * out.line_size
                    + x * match out.color_type {
                        pngc::ColorType::Grayscale => 1,
                        pngc::ColorType::GrayscaleAlpha => 2,
                        pngc::ColorType::Rgb => 3,
                        _ => 4,
                    };
                let px: [u8; 4] = match out.color_type {
                    pngc::ColorType::Grayscale => [buf[s], buf[s], buf[s], 255],
                    pngc::ColorType::GrayscaleAlpha => [buf[s], buf[s], buf[s], buf[s + 1]],
                    pngc::ColorType::Rgb => [buf[s], buf[s + 1], buf[s + 2], 255],
                    _ => [buf[s], buf[s + 1], buf[s + 2], buf[s + 3]],
                };
                let d = &mut canvas[(cy * w + cx) * 4..(cy * w + cx) * 4 + 4];
                if fc.blend_op == pngc::BlendOp::Source || px[3] == 255 {
                    d.copy_from_slice(&px);
                } else if px[3] > 0 {
                    let (sa, da) = (px[3] as f32 / 255.0, d[3] as f32 / 255.0);
                    let oa = sa + da * (1.0 - sa);
                    for c in 0..3 {
                        d[c] = ((px[c] as f32 * sa + d[c] as f32 * da * (1.0 - sa)) / oa + 0.5) as u8;
                    }
                    d[3] = (oa * 255.0 + 0.5) as u8;
                }
            }
        }
        let den = if fc.delay_den == 0 { 100 } else { fc.delay_den as u32 };
        let ms = (fc.delay_num as u32 * 1000 / den).max(10);
        check_anim_budget(pic.frames.len() + 1, w * h * 4)?;
        pic.frames.push(Frame { width: w as u32, height: h as u32, pixels: Pixels::Rgba(canvas.clone()), pos: (0, 0), delay_ms: Some(if ms < 20 { 100 } else { ms }) });
        prev = Some((fc, snapshot));
    }
    if pic.frames.is_empty() {
        bail!("APNG sans image");
    }
    let n = pic.frames.len();
    pic.frames_per_dir = n;
    let total_ms: u32 = pic.frames.iter().map(|f| f.delay_ms.unwrap_or(100)).sum();
    pic.fps = ((n as u32 * 1000) / total_ms.max(1)).max(1);
    pic.bpp = 32;
    let plays = info.animation_control.as_ref().map(|a| a.num_plays).unwrap_or(0);
    let s = pic.section("sec.format");
    s.add("p.compression", "Deflate");
    s.add("p.precision", info.bit_depth as u32);
    s.raw("Color type", color_name(info.color_type));
    s.add("p.interlaced", if info.interlaced { "p.yes" } else { "p.no" });
    s.add("p.animated", "p.yes");
    s.add("p.frames", n);
    s.raw("Loops", if plays == 0 { "infinite".to_string() } else { plays.to_string() });
    s.add("p.duration", format!("{:.2} s", total_ms as f64 / 1000.0));
    Ok(pic)
}
