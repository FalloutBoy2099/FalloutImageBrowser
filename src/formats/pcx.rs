//! ZSoft PCX : 1 / 4 / 8 bits indexés, 24 bits (3 plans), 32 bits (4 plans).

use super::*;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    if r.u8()? != 0x0A {
        bail!("signature PCX absente");
    }
    let version = r.u8()?;
    let encoding = r.u8()?;
    let bits = r.u8()? as usize;
    let (x0, y0, x1, y1) = (r.u16le()? as usize, r.u16le()? as usize, r.u16le()? as usize, r.u16le()? as usize);
    let (hdpi, vdpi) = (r.u16le()?, r.u16le()?);
    let hdr_pal = r.bytes(48)?;
    r.skip(1)?;
    let planes = r.u8()? as usize;
    let bpl = r.u16le()? as usize;
    let pal_info = r.u16le()?;
    if x1 < x0 || y1 < y0 {
        bail!("dimensions PCX invalides");
    }
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    check_dims(w as u64, h as u64)?;
    if encoding != 1 && encoding != 0 {
        bail!("encodage PCX inconnu");
    }
    let line = planes * bpl;
    if bpl == 0 || line == 0 {
        bail!("PCX invalide");
    }
    // Chaque plan doit contenir au moins `largeur × bits` bits par ligne.
    if bpl * 8 < w * bits {
        bail!("PCX : octets par ligne insuffisants pour la largeur");
    }
    let mut body_pos = 128;
    // Taille décompressée annoncée : impossible si les données sont trop courtes (une série RLE donne au plus 63 octets pour 2).
    let need = line.checked_mul(h).ok_or_else(|| anyhow!("PCX : dimensions excessives"))?;
    let have_max = data.len().saturating_sub(128).saturating_mul(if encoding == 1 { 32 } else { 1 });
    if need > have_max {
        bail!("données PCX tronquées");
    }
    let mut raw = vec![0u8; need];
    {
        let src = &data[body_pos.min(data.len())..];
        let (mut i, mut o) = (0usize, 0usize);
        while o < raw.len() {
            if i >= src.len() {
                bail!("données PCX tronquées");
            }
            let b = src[i];
            i += 1;
            if encoding == 1 && b >= 0xC0 {
                let n = (b & 0x3F) as usize;
                let v = *src.get(i).ok_or_else(|| anyhow!("données PCX tronquées"))?;
                i += 1;
                let n = n.min(raw.len() - o);
                raw[o..o + n].fill(v);
                o += n;
            } else {
                raw[o] = b;
                o += 1;
            }
        }
        body_pos += i;
    }
    let mut pic;
    match (bits, planes) {
        (8, 1) => {
            let mut palette = Palette::empty();
            let tail = &data[body_pos.min(data.len())..];
            let have = data.len() >= 769 && data[data.len() - 769] == 0x0C;
            let pal_src: Option<&[u8]> = if have {
                Some(&data[data.len() - 768..])
            } else if tail.len() >= 769 && tail[0] == 0x0C {
                Some(&tail[1..769])
            } else {
                None
            };
            for i in 0..256 {
                palette.used[i] = true;
                palette.colors[i] = match pal_src {
                    Some(p) => [p[i * 3], p[i * 3 + 1], p[i * 3 + 2], 255],
                    None => [i as u8, i as u8, i as u8, 255],
                };
            }
            let mut idx = vec![0u8; w * h];
            for y in 0..h {
                idx[y * w..(y + 1) * w].copy_from_slice(&raw[y * line..y * line + w]);
            }
            pic = Picture::single(Format::Pcx, w as u32, h as u32, Pixels::Indexed(idx), Some(palette));
        }
        (8, 3) | (8, 4) => {
            let mut out = vec![255u8; w * h * 4];
            for y in 0..h {
                for p in 0..planes {
                    let row = &raw[y * line + p * bpl..y * line + p * bpl + w];
                    for x in 0..w {
                        out[(y * w + x) * 4 + p] = row[x];
                    }
                }
            }
            pic = Picture::single(Format::Pcx, w as u32, h as u32, Pixels::Rgba(out), None);
        }
        (1, 1..=4) | (4, 1) | (2, 1) => {
            let mut palette = Palette::empty();
            for i in 0..16 {
                palette.colors[i] = [hdr_pal[i * 3], hdr_pal[i * 3 + 1], hdr_pal[i * 3 + 2], 255];
                palette.used[i] = i < (1usize << (bits * planes).min(4));
            }
            if bits == 1 && planes == 1 && hdr_pal[..6] == [0; 6] {
                palette.colors[1] = [255, 255, 255, 255];
            }
            let mut idx = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    let mut v = 0u8;
                    for p in 0..planes {
                        let b = raw[y * line + p * bpl + x * bits / 8];
                        let sh = 8 - bits - (x * bits) % 8;
                        let bitsv = (b >> sh) & ((1 << bits) - 1);
                        v |= bitsv << (p * bits);
                    }
                    idx[y * w + x] = v;
                }
            }
            pic = Picture::single(Format::Pcx, w as u32, h as u32, Pixels::Indexed(idx), Some(palette));
        }
        _ => bail!("PCX non pris en charge ({bits} bits × {planes} plans)"),
    }
    pic.bpp = (bits * planes) as u32;
    let s = pic.section("sec.format");
    s.raw(
        "Version",
        match version {
            0 => "2.5",
            2 => "2.8 (palette)",
            3 => "2.8 (sans palette)",
            4 => "Paintbrush Windows",
            5 => "3.0+",
            _ => "?",
        },
    );
    s.add("p.compression", if encoding == 1 { "RLE" } else { "p.none" });
    s.raw("Planes", planes);
    s.raw("Bits/plane", bits);
    s.raw("Bytes/line", bpl);
    s.raw("Palette info", pal_info);
    if hdpi > 0 && vdpi > 0 {
        s.add("p.dpi", format!("{hdpi} × {vdpi}"));
    }
    Ok(pic)
}

fn rle_line(out: &mut Vec<u8>, line: &[u8]) {
    let mut i = 0;
    while i < line.len() {
        let v = line[i];
        let mut n = 1;
        while i + n < line.len() && line[i + n] == v && n < 63 {
            n += 1;
        }
        if n > 1 || v >= 0xC0 {
            out.push(0xC0 | n as u8);
        }
        out.push(v);
        i += n;
    }
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    if f.width > 65535 || f.height > 65535 {
        bail!("image trop grande pour le format PCX");
    }
    let (w, h) = (f.width as usize, f.height as usize);
    let bpl = (w + 1) & !1;
    let planes = match (&f.data, p.depth) {
        (OutData::Indexed(_), _) => 1,
        (_, Depth::Rgba32) => 4,
        _ => 3,
    };
    let mut o: Vec<u8> = Vec::new();
    o.extend_from_slice(&[0x0A, 5, 1, 8]);
    put_u16le(&mut o, 0);
    put_u16le(&mut o, 0);
    put_u16le(&mut o, (w - 1) as u16);
    put_u16le(&mut o, (h - 1) as u16);
    put_u16le(&mut o, 72);
    put_u16le(&mut o, 72);
    let mut hp = [0u8; 48];
    if let Some(pal) = &p.palette {
        for i in 0..16 {
            let c = pal.colors[i];
            hp[i * 3..i * 3 + 3].copy_from_slice(&c[..3]);
        }
    }
    o.extend_from_slice(&hp);
    o.push(0);
    o.push(planes as u8);
    put_u16le(&mut o, bpl as u16);
    put_u16le(&mut o, 1);
    put_u16le(&mut o, w as u16);
    put_u16le(&mut o, h as u16);
    o.extend_from_slice(&[0u8; 54]);
    let mut line = vec![0u8; bpl * planes];
    for y in 0..h {
        line.fill(0);
        match &f.data {
            OutData::Indexed(idx) => line[..w].copy_from_slice(&idx[y * w..y * w + w]),
            OutData::Rgba(px) => {
                for x in 0..w {
                    for pl in 0..planes {
                        line[pl * bpl + x] = px[(y * w + x) * 4 + pl];
                    }
                }
            }
        }
        for pl in 0..planes {
            rle_line(&mut o, &line[pl * bpl..pl * bpl + bpl]);
        }
    }
    if let (OutData::Indexed(_), Some(pal)) = (&f.data, &p.palette) {
        o.push(0x0C);
        for i in 0..256 {
            let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
            o.extend_from_slice(&c[..3]);
        }
    }
    Ok(o)
}
