//! TIFF : lecture via la crate `tiff` (palette, gris, RVB, RVBA, CMJN...), écriture non compressée.

use super::*;
use ::tiff as tiffc;
use std::io::Cursor;
use tiffc::ColorType;
use tiffc::decoder::{Decoder, DecodingResult, Limits};
use tiffc::tags::Tag;

fn to_u8(r: DecodingResult) -> Result<(Vec<u8>, bool)> {
    Ok(match r {
        DecodingResult::U8(v) => (v, false),
        DecodingResult::U16(v) => (v.into_iter().map(|x| (x >> 8) as u8).collect(), true),
        _ => bail!("format d'échantillon TIFF non pris en charge"),
    })
}

/// La crate `tiff` ne gère pas les images à palette : on les présente comme des images en niveaux de gris
/// (indices bruts) en modifiant l'étiquette PhotometricInterpretation, puis on applique la palette.
fn patch_palette(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }
    let le = match &data[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16at = |o: usize| -> Option<u16> {
        let b = data.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32at = |o: usize| -> Option<u32> {
        let b = data.get(o..o + 4)?;
        Some(if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    let ifd = u32at(4)? as usize;
    let n = u16at(ifd)? as usize;
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        if u16at(e)? == 262 && u16at(e + 8)? == 3 {
            let mut v = data.to_vec();
            let one: [u8; 2] = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            v[e + 8..e + 10].copy_from_slice(&one);
            return Some(v);
        }
    }
    None
}

pub fn decode(orig: &[u8]) -> Result<Picture> {
    let patched = patch_palette(orig);
    let is_palette = patched.is_some();
    let data: &[u8] = patched.as_deref().unwrap_or(orig);
    let mut d = Decoder::new(Cursor::new(data)).map_err(|e| anyhow!("TIFF : {e}"))?.with_limits(Limits::unlimited());
    let (w, h) = d.dimensions().map_err(|e| anyhow!("TIFF : {e}"))?;
    check_dims(w as u64, h as u64)?;
    let (w, h) = (w as usize, h as usize);
    let ct = d.colortype().map_err(|e| anyhow!("TIFF : {e}"))?;
    let compression = d.find_tag_unsigned::<u32>(Tag::Compression).ok().flatten().unwrap_or(1);
    let photometric = if is_palette { Some(3) } else { d.find_tag_unsigned::<u32>(Tag::PhotometricInterpretation).ok().flatten() };
    let planar = d.find_tag_unsigned::<u32>(Tag::PlanarConfiguration).ok().flatten().unwrap_or(1);
    let predictor = d.find_tag_unsigned::<u32>(Tag::Predictor).ok().flatten();
    let res = |d: &mut Decoder<Cursor<&[u8]>>, t: Tag| -> Option<f64> {
        match d.find_tag(t).ok().flatten()? {
            tiffc::decoder::ifd::Value::Rational(n, dn) if dn != 0 => Some(n as f64 / dn as f64),
            _ => None,
        }
    };
    let xres = res(&mut d, Tag::XResolution);
    let yres = res(&mut d, Tag::YResolution);
    let unit = d.find_tag_unsigned::<u32>(Tag::ResolutionUnit).ok().flatten().unwrap_or(2);
    let software = d.get_tag_ascii_string(Tag::Software).ok();
    let datetime = d.get_tag_ascii_string(Tag::DateTime).ok();
    let descr = d.get_tag_ascii_string(Tag::ImageDescription).ok();
    let make = d.get_tag_ascii_string(Tag::Make).ok();
    let model = d.get_tag_ascii_string(Tag::Model).ok();
    let artist = d.get_tag_ascii_string(Tag::Artist).ok();
    let colormap = if is_palette { d.get_tag_u16_vec(Tag::ColorMap).ok() } else { None };
    let result = d.read_image().map_err(|e| anyhow!("TIFF : {e}"))?;
    let bits = match ct {
        ColorType::Gray(b) | ColorType::Palette(b) => b as usize,
        _ => 8,
    };
    let mut pages = 1;
    while d.more_images() {
        if d.next_image().is_err() {
            break;
        }
        pages += 1;
        if pages > 9999 {
            break;
        }
    }
    let (raw, was16) = to_u8(result)?;
    let mut pic;
    match ct {
        ColorType::Palette(b) | ColorType::Gray(b) if b <= 8 => {
            let stride = (w * bits).div_ceil(8);
            if raw.len() < stride * h {
                bail!("données TIFF insuffisantes");
            }
            let mut idx = vec![0u8; w * h];
            for y in 0..h {
                let row = &raw[y * stride..(y + 1) * stride];
                for x in 0..w {
                    idx[y * w + x] = match bits {
                        8 => row[x],
                        _ => {
                            let per = 8 / bits;
                            let sh = 8 - bits - (x % per) * bits;
                            (row[x / per] >> sh) & ((1 << bits) - 1)
                        }
                    };
                }
            }
            let mut pal = Palette::empty();
            let n = 1usize << bits;
            if let (true, Some(cm)) = (is_palette, &colormap) {
                let n3 = cm.len() / 3;
                for i in 0..n.min(n3).min(256) {
                    pal.colors[i] = [(cm[i] >> 8) as u8, (cm[n3 + i] >> 8) as u8, (cm[2 * n3 + i] >> 8) as u8, 255];
                    pal.used[i] = true;
                }
            } else {
                for i in 0..n.min(256) {
                    let v = (i * 255 / (n - 1).max(1)) as u8;
                    pal.colors[i] = [v, v, v, 255];
                    pal.used[i] = true;
                }
            }
            pic = Picture::single(Format::Tiff, w as u32, h as u32, Pixels::Indexed(idx), Some(pal));
            pic.bpp = bits as u32;
        }
        _ => {
            let (spp, bpp) = match ct {
                ColorType::Gray(_) => (1, 8),
                ColorType::GrayA(_) | ColorType::Multiband { num_samples: 2, .. } => (2, 16),
                ColorType::RGB(_) | ColorType::YCbCr(_) => (3, 24),
                ColorType::RGBA(_) => (4, 32),
                ColorType::CMYK(_) => (4, 32),
                ColorType::CMYKA(_) => (5, 40),
                _ => bail!("espace colorimétrique TIFF non pris en charge"),
            };
            let bpp = if was16 { bpp * 2 } else { bpp };
            if raw.len() < w * h * spp {
                bail!("données TIFF insuffisantes");
            }
            let mut out = vec![255u8; w * h * 4];
            for i in 0..w * h {
                let s = &raw[i * spp..i * spp + spp];
                let o = &mut out[i * 4..i * 4 + 4];
                match ct {
                    ColorType::Gray(_) => o[..3].fill(s[0]),
                    ColorType::GrayA(_) | ColorType::Multiband { .. } => {
                        o[..3].fill(s[0]);
                        o[3] = s[1];
                    }
                    ColorType::RGB(_) | ColorType::YCbCr(_) => o[..3].copy_from_slice(&s[..3]),
                    ColorType::RGBA(_) => o.copy_from_slice(&s[..4]),
                    _ => {
                        let k = s[3] as u32;
                        for c in 0..3 {
                            o[c] = ((255 - s[c] as u32) * (255 - k) / 255) as u8;
                        }
                        if spp == 5 {
                            o[3] = s[4];
                        }
                    }
                }
            }
            pic = Picture::single(Format::Tiff, w as u32, h as u32, Pixels::Rgba(out), None);
            pic.bpp = bpp as u32;
        }
    }
    let s = pic.section("sec.format");
    s.add(
        "p.compression",
        match compression {
            1 => "p.none",
            2 => "CCITT RLE",
            3 => "CCITT Group 3",
            4 => "CCITT Group 4",
            5 => "LZW",
            6 | 7 => "JPEG",
            8 | 32946 => "Deflate",
            32773 => "PackBits",
            _ => "?",
        },
    );
    if let Some(p) = photometric {
        s.raw(
            "Photometric",
            match p {
                0 => "WhiteIsZero",
                1 => "BlackIsZero",
                2 => "RGB",
                3 => "Palette",
                5 => "CMYK",
                6 => "YCbCr",
                8 => "CIE Lab",
                _ => "?",
            },
        );
    }
    s.raw("Planar configuration", planar);
    if let Some(p) = predictor {
        s.raw("Predictor", p);
    }
    s.raw("Color type", format!("{ct:?}"));
    if let (Some(x), Some(y)) = (xres, yres) {
        let f = if unit == 3 { 2.54 } else { 1.0 };
        s.add("p.dpi", format!("{:.0} × {:.0}", x * f, y * f));
    }
    s.add("p.pages", pages);
    for (k, v) in [("Software", software), ("Date", datetime), ("Description", descr), ("Make", make), ("Model", model), ("Artist", artist)] {
        if let Some(v) = v {
            s.raw(k, v);
        }
    }
    if let Some(e) = exif_section(data) {
        pic.sections.push(e);
    }
    Ok(pic)
}

fn short(v: u16) -> [u8; 4] {
    let b = v.to_le_bytes();
    [b[0], b[1], 0, 0]
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let (w, h) = (f.width, f.height);
    let mut o: Vec<u8> = Vec::new();
    o.extend_from_slice(&[b'I', b'I', 42, 0]);
    put_u32le(&mut o, 0); // décalage de l'IFD, complété plus bas
    let pix_off = o.len() as u32;
    let (spp, bps, photo, extra): (u16, u16, u16, bool);
    match &f.data {
        OutData::Indexed(idx) => {
            o.extend_from_slice(idx);
            (spp, bps, photo, extra) = (1, 8, 3, false);
        }
        OutData::Rgba(px) => {
            if p.depth == Depth::Rgba32 {
                o.extend_from_slice(px);
                (spp, bps, photo, extra) = (4, 8, 2, true);
            } else {
                for c in px.chunks_exact(4) {
                    o.extend_from_slice(&c[..3]);
                }
                (spp, bps, photo, extra) = (3, 8, 2, false);
            }
        }
    }
    let pix_len = o.len() as u32 - pix_off;
    if o.len() & 1 == 1 {
        o.push(0);
    }
    // Données hors IFD.
    let bps_off = o.len() as u32;
    if spp > 2 {
        for _ in 0..spp {
            put_u16le(&mut o, bps);
        }
    }
    let cmap_off = o.len() as u32;
    if let (OutData::Indexed(_), Some(pal)) = (&f.data, &p.palette) {
        for ch in 0..3 {
            for i in 0..256 {
                let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
                put_u16le(&mut o, (c[ch] as u16) << 8 | c[ch] as u16);
            }
        }
    }
    let ifd_off = o.len() as u32;
    o[4..8].copy_from_slice(&ifd_off.to_le_bytes());
    // (tag, type, count, value)
    let mut e: Vec<(u16, u16, u32, [u8; 4])> = vec![
        (256, 4, 1, w.to_le_bytes()),
        (257, 4, 1, h.to_le_bytes()),
        (258, 3, spp as u32, if spp > 2 { bps_off.to_le_bytes() } else { short(bps) }),
        (259, 3, 1, short(1)),
        (262, 3, 1, short(photo)),
        (273, 4, 1, pix_off.to_le_bytes()),
        (277, 3, 1, short(spp)),
        (278, 4, 1, h.to_le_bytes()),
        (279, 4, 1, pix_len.to_le_bytes()),
        (284, 3, 1, short(1)),
    ];
    if spp == 2 {
        // non utilisé : cas gris+alpha non produit
    }
    if photo == 3 {
        e.push((320, 3, 768, cmap_off.to_le_bytes()));
    }
    if extra {
        e.push((338, 3, 1, short(2)));
    }
    e.sort_by_key(|t| t.0);
    put_u16le(&mut o, e.len() as u16);
    for (tag, ty, cnt, val) in e {
        put_u16le(&mut o, tag);
        put_u16le(&mut o, ty);
        put_u32le(&mut o, cnt);
        o.extend_from_slice(&val);
    }
    put_u32le(&mut o, 0);
    Ok(o)
}
