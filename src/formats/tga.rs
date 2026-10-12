//! Truevision TGA : palette 8 bits, gris, 16 / 24 / 32 bits, avec ou sans RLE.

use super::*;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    let id_len = r.u8()? as usize;
    let cmap_type = r.u8()?;
    let img_type = r.u8()?;
    let cmap_first = r.u16le()? as usize;
    let cmap_len = r.u16le()? as usize;
    let cmap_bits = r.u8()?;
    let (xo, yo) = (r.u16le()?, r.u16le()?);
    let w = r.u16le()? as usize;
    let h = r.u16le()? as usize;
    let bpp = r.u8()? as usize;
    let desc = r.u8()?;
    check_dims(w as u64, h as u64)?;
    if !matches!(img_type, 1 | 2 | 3 | 9 | 10 | 11) {
        bail!("type d'image TGA non pris en charge ({img_type})");
    }
    let id = r.bytes(id_len)?;
    let rle = img_type >= 9;
    let base = if rle { img_type - 8 } else { img_type };
    let mut palette = None;
    if cmap_type == 1 {
        let eb = (cmap_bits as usize).div_ceil(8);
        let raw = r.bytes(cmap_len * eb)?;
        let mut p = Palette::empty();
        for i in 0..cmap_len {
            let e = &raw[i * eb..i * eb + eb];
            let c = match cmap_bits {
                15 | 16 => {
                    let v = u16::from_le_bytes([e[0], e[1]]);
                    let (r5, g5, b5) = ((v >> 10) & 31, (v >> 5) & 31, v & 31);
                    [exp5(r5), exp5(g5), exp5(b5), 255]
                }
                24 => [e[2], e[1], e[0], 255],
                32 => [e[2], e[1], e[0], 255],
                _ => bail!("palette TGA {cmap_bits} bits non prise en charge"),
            };
            if let Some(slot) = (i + cmap_first < 256).then_some(i + cmap_first) {
                p.colors[slot] = c;
                p.used[slot] = true;
            }
        }
        palette = Some(p);
    }
    let bytes_pp = bpp.div_ceil(8);
    if bytes_pp == 0 || bytes_pp > 4 {
        bail!("profondeur TGA non prise en charge ({bpp} bits)");
    }
    // Pixels bruts dans l'ordre du fichier.
    let n = w * h;
    let mut raw = vec![0u8; n * bytes_pp];
    if !rle {
        let src = r.bytes(n * bytes_pp)?;
        raw.copy_from_slice(src);
    } else {
        let mut o = 0usize;
        while o < raw.len() {
            let c = r.u8()?;
            let cnt = (c & 0x7F) as usize + 1;
            let take = (cnt * bytes_pp).min(raw.len() - o);
            if c & 0x80 != 0 {
                let px = r.bytes(bytes_pp)?;
                for k in 0..take / bytes_pp {
                    raw[o + k * bytes_pp..o + (k + 1) * bytes_pp].copy_from_slice(px);
                }
            } else {
                let src = r.bytes(cnt * bytes_pp)?;
                raw[o..o + take].copy_from_slice(&src[..take]);
            }
            o += take;
        }
    }
    let top = desc & 0x20 != 0;
    let right = desc & 0x10 != 0;
    let alpha_bits = desc & 0x0F;
    let mut pic;
    let place = |x: usize, y: usize| -> usize {
        let dy = if top { y } else { h - 1 - y };
        let dx = if right { w - 1 - x } else { x };
        dy * w + dx
    };
    match (base, bpp) {
        (1, 8) | (3, 8) => {
            let mut idx = vec![0u8; n];
            for y in 0..h {
                for x in 0..w {
                    idx[place(x, y)] = raw[y * w + x];
                }
            }
            let pal = if base == 3 {
                let mut p = Palette::empty();
                for i in 0..256 {
                    p.colors[i] = [i as u8, i as u8, i as u8, 255];
                    p.used[i] = true;
                }
                Some(p)
            } else {
                if cmap_first != 0 {
                    for v in idx.iter_mut() {
                        *v = v.wrapping_sub(cmap_first as u8);
                    }
                }
                palette
            };
            pic = Picture::single(Format::Tga, w as u32, h as u32, Pixels::Indexed(idx), pal);
        }
        (2, 15) | (2, 16) | (2, 24) | (2, 32) | (3, 16) => {
            let mut out = vec![255u8; n * 4];
            for y in 0..h {
                for x in 0..w {
                    let s = &raw[(y * w + x) * bytes_pp..(y * w + x + 1) * bytes_pp];
                    let o = place(x, y) * 4;
                    match (base, bpp) {
                        (3, 16) => {
                            out[o..o + 3].fill(s[0]);
                            out[o + 3] = s[1];
                        }
                        (_, 15) | (_, 16) => {
                            let v = u16::from_le_bytes([s[0], s[1]]);
                            out[o] = exp5((v >> 10) & 31);
                            out[o + 1] = exp5((v >> 5) & 31);
                            out[o + 2] = exp5(v & 31);
                            out[o + 3] = if bpp == 16 && alpha_bits > 0 && v & 0x8000 == 0 { 0 } else { 255 };
                        }
                        (_, 24) => {
                            out[o] = s[2];
                            out[o + 1] = s[1];
                            out[o + 2] = s[0];
                        }
                        _ => {
                            out[o] = s[2];
                            out[o + 1] = s[1];
                            out[o + 2] = s[0];
                            out[o + 3] = if alpha_bits > 0 { s[3] } else { 255 };
                        }
                    }
                }
            }
            pic = Picture::single(Format::Tga, w as u32, h as u32, Pixels::Rgba(out), None);
        }
        _ => bail!("combinaison TGA non prise en charge (type {img_type}, {bpp} bits)"),
    }
    pic.bpp = bpp as u32;
    let s = pic.section("sec.format");
    s.raw(
        "Type",
        match base {
            1 => "Color-mapped",
            2 => "True-color",
            _ => "Grayscale",
        },
    );
    s.add("p.compression", if rle { "RLE" } else { "p.none" });
    s.add("p.orientation", if top { "p.top_down" } else { "p.bottom_up" });
    s.add("p.alpha_bits", alpha_bits);
    if cmap_type == 1 {
        s.add("p.palette_entries", cmap_len);
    }
    if xo != 0 || yo != 0 {
        s.raw("Origin", format!("{xo}, {yo}"));
    }
    if !id.is_empty() {
        s.raw("Image ID", String::from_utf8_lossy(id).trim_end_matches('\0').to_string());
    }
    Ok(pic)
}

fn exp5(v: u16) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    if f.width > 65535 || f.height > 65535 {
        bail!("image trop grande pour le format TGA");
    }
    let mut o: Vec<u8> = Vec::new();
    let (w, h) = (f.width as usize, f.height as usize);
    let indexed = matches!(f.data, OutData::Indexed(_));
    let (img_type, bpp, desc) = if indexed {
        (1u8, 8u8, 0x20u8)
    } else {
        match p.depth {
            Depth::Rgb16 => (2, 16, 0x21),
            Depth::Rgb24 => (2, 24, 0x20),
            _ => (2, 32, 0x28),
        }
    };
    o.push(0);
    o.push(indexed as u8);
    o.push(img_type);
    put_u16le(&mut o, 0);
    put_u16le(&mut o, if indexed { 256 } else { 0 });
    o.push(if indexed { 24 } else { 0 });
    put_u16le(&mut o, 0);
    put_u16le(&mut o, 0);
    put_u16le(&mut o, w as u16);
    put_u16le(&mut o, h as u16);
    o.push(bpp);
    o.push(desc);
    match (&f.data, &p.palette) {
        (OutData::Indexed(idx), Some(pal)) => {
            for i in 0..256 {
                let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
                o.extend_from_slice(&[c[2], c[1], c[0]]);
            }
            o.extend_from_slice(idx);
        }
        (OutData::Rgba(px), _) => {
            for c in px.chunks_exact(4) {
                match bpp {
                    16 => {
                        let v = ((c[0] as u16 >> 3) << 10) | ((c[1] as u16 >> 3) << 5) | (c[2] as u16 >> 3) | if c[3] >= 128 { 0x8000 } else { 0 };
                        put_u16le(&mut o, v);
                    }
                    24 => o.extend_from_slice(&[c[2], c[1], c[0]]),
                    _ => o.extend_from_slice(&[c[2], c[1], c[0], c[3]]),
                }
            }
        }
        _ => bail!("palette manquante"),
    }
    o.extend_from_slice(b"\0\0\0\0\0\0\0\0TRUEVISION-XFILE.\0");
    Ok(o)
}
