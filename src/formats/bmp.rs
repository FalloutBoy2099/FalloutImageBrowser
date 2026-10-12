//! BMP (Windows / OS/2) : 1, 4, 8 (RLE compris), 16, 24 et 32 bits.

use super::*;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    if r.bytes(2)? != b"BM" {
        bail!("signature BMP absente");
    }
    r.skip(8)?;
    let off_bits = r.u32le()? as usize;
    let dib = r.u32le()? as usize;
    let (w, h_raw, bpp, comp, clr_used, xppm, yppm);
    let mut masks = [0u32; 4];
    let entry_size;
    let mut has_alpha_mask = false;
    if dib == 12 {
        w = r.u16le()? as i32;
        h_raw = r.u16le()? as i16 as i32;
        r.skip(2)?;
        bpp = r.u16le()? as u32;
        comp = 0;
        clr_used = 0;
        xppm = 0;
        yppm = 0;
        entry_size = 3;
    } else if dib >= 40 {
        w = r.i32le()?;
        h_raw = r.i32le()?;
        r.skip(2)?;
        bpp = r.u16le()? as u32;
        comp = r.u32le()?;
        r.skip(4)?;
        xppm = r.u32le()?;
        yppm = r.u32le()?;
        clr_used = r.u32le()?;
        r.skip(4)?;
        if dib >= 52 {
            for m in masks.iter_mut().take(3) {
                *m = r.u32le()?;
            }
        }
        if dib >= 56 {
            masks[3] = r.u32le()?;
            has_alpha_mask = true;
        }
        entry_size = 4;
    } else {
        bail!("en-tête BMP non pris en charge (taille {dib})");
    }
    if w <= 0 || h_raw == 0 {
        bail!("dimensions BMP invalides");
    }
    let bottom_up = h_raw > 0;
    let (w, h) = (w as usize, h_raw.unsigned_abs() as usize);
    check_dims(w as u64, h as u64)?;
    let mut pal_pos = 14 + dib;
    if dib == 40 && (comp == 3 || comp == 6) {
        let mut rr = Rd::at(data, 54);
        for m in masks.iter_mut().take(if comp == 6 { 4 } else { 3 }) {
            *m = rr.u32le()?;
        }
        has_alpha_mask = comp == 6;
        pal_pos = 54 + if comp == 6 { 16 } else { 12 };
    }
    let comp_name = match comp {
        0 => "BI_RGB",
        1 => "BI_RLE8",
        2 => "BI_RLE4",
        3 => "BI_BITFIELDS",
        4 => "BI_JPEG",
        5 => "BI_PNG",
        6 => "BI_ALPHABITFIELDS",
        _ => "?",
    };
    if comp == 4 || comp == 5 {
        bail!("BMP avec compression {comp_name} non pris en charge");
    }
    let mut palette = None;
    if bpp <= 8 {
        let n = if clr_used > 0 { clr_used as usize } else { 1usize << bpp }.min(256);
        let mut p = Palette::empty();
        let mut rr = Rd::at(data, pal_pos);
        for i in 0..n {
            let e = rr.bytes(entry_size)?;
            p.colors[i] = [e[2], e[1], e[0], 255];
            p.used[i] = true;
        }
        palette = Some(p);
    }
    if off_bits > data.len() {
        bail!("décalage des pixels invalide");
    }
    let px = &data[off_bits..];
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    let mut pic;
    match bpp {
        1 | 4 | 8 => {
            let mut idx = vec![0u8; w * h];
            if comp == 1 || comp == 2 {
                decode_rle(px, w, h, comp == 1, &mut idx);
            } else {
                if px.len() < stride * h {
                    bail!("données BMP tronquées");
                }
                for y in 0..h {
                    let row = &px[y * stride..y * stride + stride];
                    let dy = if bottom_up { h - 1 - y } else { y };
                    let out = &mut idx[dy * w..dy * w + w];
                    match bpp {
                        8 => out.copy_from_slice(&row[..w]),
                        4 => {
                            for x in 0..w {
                                let b = row[x / 2];
                                out[x] = if x % 2 == 0 { b >> 4 } else { b & 15 };
                            }
                        }
                        _ => {
                            for x in 0..w {
                                out[x] = (row[x / 8] >> (7 - x % 8)) & 1;
                            }
                        }
                    }
                }
            }
            if (comp == 1 || comp == 2) && bottom_up {
                flip_rows(&mut idx, w, h);
            }
            pic = Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Indexed(idx), palette);
        }
        16 | 24 | 32 => {
            if px.len() < stride * h {
                bail!("données BMP tronquées");
            }
            let (mut mr, mut mg, mut mb, mut ma) = (masks[0], masks[1], masks[2], masks[3]);
            if comp == 0 || (mr | mg | mb) == 0 {
                if bpp == 16 {
                    (mr, mg, mb) = (0x7C00, 0x03E0, 0x001F);
                } else if bpp == 32 {
                    (mr, mg, mb) = (0x00FF0000, 0x0000FF00, 0x000000FF);
                    if comp == 0 && !has_alpha_mask {
                        ma = 0xFF000000;
                    }
                }
            }
            let mut out = vec![255u8; w * h * 4];
            // Pour 32 bits sans masque d'alpha explicite : l'alpha n'est utilisé que s'il est renseigné.
            let mut any_alpha = false;
            for y in 0..h {
                let row = &px[y * stride..y * stride + stride];
                let dy = if bottom_up { h - 1 - y } else { y };
                let o = &mut out[dy * w * 4..(dy + 1) * w * 4];
                match bpp {
                    24 => {
                        for x in 0..w {
                            o[x * 4] = row[x * 3 + 2];
                            o[x * 4 + 1] = row[x * 3 + 1];
                            o[x * 4 + 2] = row[x * 3];
                        }
                    }
                    16 => {
                        for x in 0..w {
                            let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32;
                            o[x * 4] = extract(v, mr);
                            o[x * 4 + 1] = extract(v, mg);
                            o[x * 4 + 2] = extract(v, mb);
                            if ma != 0 {
                                o[x * 4 + 3] = extract(v, ma);
                            }
                        }
                    }
                    _ => {
                        for x in 0..w {
                            let v = u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]]);
                            o[x * 4] = extract(v, mr);
                            o[x * 4 + 1] = extract(v, mg);
                            o[x * 4 + 2] = extract(v, mb);
                            if ma != 0 {
                                let a = extract(v, ma);
                                any_alpha |= a != 0;
                                o[x * 4 + 3] = a;
                            }
                        }
                    }
                }
            }
            if bpp == 32 && ma != 0 && !any_alpha && !has_alpha_mask {
                for p in out.chunks_exact_mut(4) {
                    p[3] = 255;
                }
            }
            pic = Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(out), None);
        }
        _ => bail!("profondeur BMP non prise en charge ({bpp} bits)"),
    }
    pic.bpp = bpp;
    let s = pic.section("sec.format");
    s.raw(
        "Header",
        match dib {
            12 => "BITMAPCOREHEADER (OS/2)".to_string(),
            40 => "BITMAPINFOHEADER".to_string(),
            52 => "BITMAPV2INFOHEADER".to_string(),
            56 => "BITMAPV3INFOHEADER".to_string(),
            108 => "BITMAPV4HEADER".to_string(),
            124 => "BITMAPV5HEADER".to_string(),
            n => format!("{n}"),
        },
    );
    s.add("p.compression", comp_name);
    s.add("p.orientation", if bottom_up { "p.bottom_up" } else { "p.top_down" });
    if bpp <= 8 {
        s.add("p.palette_entries", clr_used.max(if clr_used == 0 { 1 << bpp } else { 0 }).min(256));
    }
    if xppm > 0 && yppm > 0 {
        s.add("p.dpi", format!("{} × {}", dpi_from_ppm(xppm), dpi_from_ppm(yppm)));
    }
    if matches!(bpp, 16) {
        s.raw("Masks", format!("R {:#06X}  G {:#06X}  B {:#06X}", masks[0], masks[1], masks[2]));
    }
    Ok(pic)
}

fn extract(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let sh = mask.trailing_zeros();
    let m = mask >> sh;
    let bits = 32 - m.leading_zeros();
    let x = (v & mask) >> sh;
    if bits >= 8 { (x >> (bits - 8)) as u8 } else { (x * 255 / ((1u32 << bits) - 1)) as u8 }
}

fn flip_rows(buf: &mut [u8], w: usize, h: usize) {
    for y in 0..h / 2 {
        let (a, b) = buf.split_at_mut((h - 1 - y) * w);
        a[y * w..y * w + w].swap_with_slice(&mut b[..w]);
    }
}

/// Décode RLE4 / RLE8 ; l'image est produite dans l'ordre du fichier (première ligne = bas).
fn decode_rle(src: &[u8], w: usize, h: usize, rle8: bool, out: &mut [u8]) {
    let (mut x, mut y, mut i) = (0usize, 0usize, 0usize);
    let put = |x: usize, y: usize, v: u8, out: &mut [u8]| {
        if x < w && y < h {
            out[y * w + x] = v;
        }
    };
    while i + 1 < src.len() && y < h {
        let (n, c) = (src[i] as usize, src[i + 1]);
        i += 2;
        if n > 0 {
            for k in 0..n {
                let v = if rle8 {
                    c
                } else if k % 2 == 0 {
                    c >> 4
                } else {
                    c & 15
                };
                put(x, y, v, out);
                x += 1;
            }
        } else {
            match c {
                0 => {
                    x = 0;
                    y += 1;
                }
                1 => break,
                2 => {
                    if i + 1 >= src.len() {
                        break;
                    }
                    x += src[i] as usize;
                    y += src[i + 1] as usize;
                    i += 2;
                }
                n => {
                    let n = n as usize;
                    let bytes = if rle8 { n } else { n.div_ceil(2) };
                    for k in 0..n {
                        let v = if rle8 {
                            src.get(i + k).copied().unwrap_or(0)
                        } else {
                            let b = src.get(i + k / 2).copied().unwrap_or(0);
                            if k % 2 == 0 { b >> 4 } else { b & 15 }
                        };
                        put(x, y, v, out);
                        x += 1;
                    }
                    i += bytes + (bytes & 1);
                }
            }
        }
    }
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let (w, h) = (f.width as usize, f.height as usize);
    let mut out: Vec<u8> = Vec::new();
    let (bpp, pal_bytes, dib_size, comp): (u16, usize, u32, u32) = match (&f.data, p.depth) {
        (OutData::Indexed(_), _) => (8, 1024, 40, 0),
        (_, Depth::Rgb16) => (16, 0, 40, 0),
        (_, Depth::Rgb24) => (24, 0, 40, 0),
        _ => (32, 0, 108, 3),
    };
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    let off = 14 + dib_size as usize + pal_bytes;
    let size = off + stride * h;
    out.extend_from_slice(b"BM");
    put_u32le(&mut out, size as u32);
    put_u32le(&mut out, 0);
    put_u32le(&mut out, off as u32);
    put_u32le(&mut out, dib_size);
    put_u32le(&mut out, w as u32);
    put_u32le(&mut out, h as u32);
    put_u16le(&mut out, 1);
    put_u16le(&mut out, bpp);
    put_u32le(&mut out, comp);
    put_u32le(&mut out, (stride * h) as u32);
    put_u32le(&mut out, 2835);
    put_u32le(&mut out, 2835);
    put_u32le(&mut out, if bpp == 8 { 256 } else { 0 });
    put_u32le(&mut out, 0);
    if dib_size == 108 {
        for m in [0x00FF0000u32, 0x0000FF00, 0x000000FF, 0xFF000000] {
            put_u32le(&mut out, m);
        }
        put_u32le(&mut out, 0x73524742); // "sRGB"
        out.extend_from_slice(&[0u8; 36 + 12]);
    }
    if let (OutData::Indexed(_), Some(pal)) = (&f.data, &p.palette) {
        for i in 0..256 {
            let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
            out.extend_from_slice(&[c[2], c[1], c[0], 0]);
        }
    }
    let mut row = vec![0u8; stride];
    for y in (0..h).rev() {
        row.fill(0);
        match &f.data {
            OutData::Indexed(idx) => row[..w].copy_from_slice(&idx[y * w..y * w + w]),
            OutData::Rgba(px) => {
                let src = &px[y * w * 4..(y + 1) * w * 4];
                match bpp {
                    16 => {
                        for x in 0..w {
                            let v = ((src[x * 4] as u16 >> 3) << 10) | ((src[x * 4 + 1] as u16 >> 3) << 5) | (src[x * 4 + 2] as u16 >> 3);
                            row[x * 2..x * 2 + 2].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                    24 => {
                        for x in 0..w {
                            row[x * 3] = src[x * 4 + 2];
                            row[x * 3 + 1] = src[x * 4 + 1];
                            row[x * 3 + 2] = src[x * 4];
                        }
                    }
                    _ => {
                        for x in 0..w {
                            row[x * 4] = src[x * 4 + 2];
                            row[x * 4 + 1] = src[x * 4 + 1];
                            row[x * 4 + 2] = src[x * 4];
                            row[x * 4 + 3] = src[x * 4 + 3];
                        }
                    }
                }
            }
        }
        out.extend_from_slice(&row);
    }
    Ok(out)
}
