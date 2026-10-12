//! DDS (DirectDraw Surface) : lecture de la première surface (BC1 à BC7, formats non compressés, en-tête DX10).

use super::*;

fn le32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0)
}

/// Valeur d'un canal 8 bits extraite d'un pixel selon son masque.
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

fn half_to_u8(h: u16) -> u8 {
    let (s, e, m) = ((h >> 15) & 1, (h >> 10) & 31, (h & 1023) as f32);
    let v = match e {
        0 => m / 1024.0 * 2f32.powi(-14),
        31 => 1.0,
        _ => (1.0 + m / 1024.0) * 2f32.powi(e as i32 - 15),
    };
    let v = if s == 1 { 0.0 } else { v.clamp(0.0, 1.0) };
    let g = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (g * 255.0 + 0.5) as u8
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Bc {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc6U,
    Bc6S,
    Bc7,
}

fn bc_name(b: Bc) -> &'static str {
    match b {
        Bc::Bc1 => "BC1 (DXT1)",
        Bc::Bc2 => "BC2 (DXT3)",
        Bc::Bc3 => "BC3 (DXT5)",
        Bc::Bc4 => "BC4 (ATI1)",
        Bc::Bc5 => "BC5 (ATI2)",
        Bc::Bc6U | Bc::Bc6S => "BC6H",
        Bc::Bc7 => "BC7",
    }
}

fn decode_bc(b: Bc, data: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let block = match b {
        Bc::Bc1 | Bc::Bc4 => 8,
        _ => 16,
    };
    let need = w.div_ceil(4) * h.div_ceil(4) * block;
    if data.len() < need {
        bail!("DDS : données insuffisantes ({} octets, {need} attendus)", data.len());
    }
    let mut px = vec![0u32; w.div_ceil(4) * 4 * h.div_ceil(4) * 4];
    let r = match b {
        Bc::Bc1 => texture2ddecoder::decode_bc1a(data, w, h, &mut px),
        Bc::Bc2 => texture2ddecoder::decode_bc2(data, w, h, &mut px),
        Bc::Bc3 => texture2ddecoder::decode_bc3(data, w, h, &mut px),
        Bc::Bc4 => texture2ddecoder::decode_bc4(data, w, h, &mut px),
        Bc::Bc5 => texture2ddecoder::decode_bc5(data, w, h, &mut px),
        Bc::Bc6U => texture2ddecoder::decode_bc6(data, w, h, &mut px, false),
        Bc::Bc6S => texture2ddecoder::decode_bc6(data, w, h, &mut px, true),
        Bc::Bc7 => texture2ddecoder::decode_bc7(data, w, h, &mut px),
    };
    r.map_err(|e| anyhow!("DDS : {e}"))?;
    let mut out = vec![255u8; w * h * 4];
    for i in 0..w * h {
        let [bb, g, rr, a] = px[i].to_le_bytes();
        let o = i * 4;
        match b {
            Bc::Bc4 => out[o..o + 3].fill(rr),
            Bc::Bc5 => {
                // Cartes de normales : le bleu est reconstruit à partir des deux composantes.
                let (x, y) = (rr as f32 / 127.5 - 1.0, g as f32 / 127.5 - 1.0);
                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                out[o] = rr;
                out[o + 1] = g;
                out[o + 2] = ((z + 1.0) * 127.5) as u8;
            }
            _ => {
                out[o] = rr;
                out[o + 1] = g;
                out[o + 2] = bb;
                out[o + 3] = if matches!(b, Bc::Bc6U | Bc::Bc6S) { 255 } else { a };
            }
        }
    }
    Ok(out)
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    if data.len() < 128 || &data[..4] != b"DDS " || le32(data, 4) != 124 {
        bail!("signature ou en-tête DDS invalide");
    }
    let (flags, h, w, mips) = (le32(data, 8), le32(data, 12) as usize, le32(data, 16) as usize, le32(data, 28));
    check_dims(w as u64, h as u64)?;
    let (pf_flags, fourcc, bits) = (le32(data, 80), &data[84..88], le32(data, 88) as usize);
    let (rm, gm, bm, am) = (le32(data, 92), le32(data, 96), le32(data, 100), le32(data, 104));
    let (caps2, depth) = (le32(data, 112), le32(data, 24));
    let mut start = 128usize;
    let mut dxgi: Option<u32> = None;
    let mut bc: Option<Bc> = None;
    let mut label = String::new();
    let mut premultiplied = false;
    if pf_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => bc = Some(Bc::Bc1),
            b"DXT2" | b"DXT3" => {
                bc = Some(Bc::Bc2);
                premultiplied = fourcc == b"DXT2";
            }
            b"DXT4" | b"DXT5" => {
                bc = Some(Bc::Bc3);
                premultiplied = fourcc == b"DXT4";
            }
            b"ATI1" | b"BC4U" => bc = Some(Bc::Bc4),
            b"ATI2" | b"BC5U" => bc = Some(Bc::Bc5),
            b"DX10" => {
                dxgi = Some(le32(data, 128));
                start = 148;
                bc = match dxgi.unwrap() {
                    71 | 72 => Some(Bc::Bc1),
                    74 | 75 => Some(Bc::Bc2),
                    77 | 78 => Some(Bc::Bc3),
                    80 | 81 => Some(Bc::Bc4),
                    83 | 84 => Some(Bc::Bc5),
                    95 => Some(Bc::Bc6U),
                    96 => Some(Bc::Bc6S),
                    98 | 99 => Some(Bc::Bc7),
                    _ => None,
                };
            }
            _ => {}
        }
        label = String::from_utf8_lossy(fourcc).to_string();
    }
    let body = data.get(start..).unwrap_or(&[]);
    let rgba: Vec<u8> = if let Some(b) = bc {
        label = format!("{} — {}", label, bc_name(b));
        decode_bc(b, body, w, h)?
    } else if let Some(f) = dxgi {
        // Formats DXGI non compressés les plus courants.
        let (bpp, conv): (usize, fn(&[u8]) -> [u8; 4]) = match f {
            28 | 29 => (4, |p| [p[0], p[1], p[2], p[3]]),
            87 | 91 => (4, |p| [p[2], p[1], p[0], p[3]]),
            88 | 93 => (4, |p| [p[2], p[1], p[0], 255]),
            61 => (1, |p| [p[0], p[0], p[0], 255]),
            65 => (1, |p| [255, 255, 255, p[0]]),
            56 => (2, |p| [p[1], p[1], p[1], 255]),
            49 => (2, |p| [p[0], p[1], 0, 255]),
            24 => (4, |p| {
                let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                [((v & 1023) >> 2) as u8, (((v >> 10) & 1023) >> 2) as u8, (((v >> 20) & 1023) >> 2) as u8, ((v >> 30) as u8) * 85]
            }),
            11 => (8, |p| [p[1], p[3], p[5], p[7]]),
            10 => (8, |p| {
                let h = |i: usize| half_to_u8(u16::from_le_bytes([p[i], p[i + 1]]));
                [h(0), h(2), h(4), (u16::from_le_bytes([p[6], p[7]]) as f32 / 15360.0).clamp(0.0, 1.0).mul_add(255.0, 0.5) as u8]
            }),
            f => bail!("DDS : format DXGI {f} non pris en charge"),
        };
        label = format!("DXGI {f}");
        if body.len() < w * h * bpp {
            bail!("DDS : données insuffisantes");
        }
        let mut out = vec![0u8; w * h * 4];
        for i in 0..w * h {
            out[i * 4..i * 4 + 4].copy_from_slice(&conv(&body[i * bpp..i * bpp + bpp]));
        }
        out
    } else if pf_flags & 0x4 != 0 {
        bail!("DDS : format « {} » non pris en charge", String::from_utf8_lossy(fourcc));
    } else {
        // Non compressé avec masques de bits (RVB, RVBA, luminance, alpha seul).
        let bytes = bits / 8;
        if !matches!(bytes, 1..=4) || bits % 8 != 0 {
            bail!("DDS : {bits} bits par pixel non pris en charge");
        }
        if body.len() < w * h * bytes {
            bail!("DDS : données insuffisantes");
        }
        let lum = pf_flags & 0x20000 != 0;
        let alpha_only = pf_flags & 0x2 != 0 && pf_flags & 0x41 == 0;
        let has_alpha = pf_flags & 0x1 != 0 && am != 0;
        label = format!(
            "{bits}-bit {}",
            if lum {
                "luminance"
            } else if alpha_only {
                "alpha"
            } else {
                "RGB"
            }
        );
        let mut out = vec![255u8; w * h * 4];
        for i in 0..w * h {
            let mut v = 0u32;
            for k in 0..bytes {
                v |= (body[i * bytes + k] as u32) << (8 * k);
            }
            let o = i * 4;
            if lum {
                let l = extract(v, rm);
                out[o..o + 3].fill(l);
            } else if alpha_only {
                out[o..o + 3].fill(255);
                out[o + 3] = extract(v, am);
                continue;
            } else {
                out[o] = extract(v, rm);
                out[o + 1] = extract(v, gm);
                out[o + 2] = extract(v, bm);
            }
            if has_alpha {
                out[o + 3] = extract(v, am);
            }
        }
        out
    };
    let mut pic = Picture::single(Format::Dds, w as u32, h as u32, Pixels::Rgba(rgba), None);
    pic.bpp = if bc == Some(Bc::Bc1) || bc == Some(Bc::Bc4) {
        4
    } else if bc.is_some() {
        8
    } else {
        bits.max(8) as u32
    };
    let s = pic.section("sec.format");
    s.add("p.compression", label);
    s.raw("Mipmaps", mips.max(1));
    if flags & 0x800000 != 0 && depth > 1 {
        s.raw("Depth (volume, first slice shown)", depth);
    }
    if caps2 & 0x200 != 0 {
        s.raw("Cubemap", "first face shown");
    }
    if premultiplied {
        s.raw("Alpha", "premultiplied");
    }
    if let Some(f) = dxgi {
        s.raw("DXGI format", f);
    }
    Ok(pic)
}

/// Écriture DDS non compressée : A8R8G8B8 (32 bits), R8G8B8 (24 bits) ou A1R5G5B5 (16 bits), une seule surface, sans mipmaps.
pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let OutData::Rgba(px) = &f.data else { bail!("DDS : image RVB requise") };
    let (bytes, flags, (rm, gm, bm, am)): (usize, u32, (u32, u32, u32, u32)) = match p.depth {
        Depth::Rgba32 => (4, 0x41, (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000)),
        Depth::Rgb16 => (2, 0x41, (0x7C00, 0x03E0, 0x001F, 0x8000)),
        _ => (3, 0x40, (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0)),
    };
    let (w, h) = (f.width as usize, f.height as usize);
    let mut o: Vec<u8> = Vec::with_capacity(128 + w * h * bytes);
    o.extend_from_slice(b"DDS ");
    for v in [124u32, 0x100F, h as u32, w as u32, (w * bytes) as u32, 0, 0] {
        o.extend_from_slice(&v.to_le_bytes());
    }
    o.extend_from_slice(&[0u8; 44]); // réservé (11 × u32)
    for v in [32u32, flags, 0, (bytes * 8) as u32, rm, gm, bm, am, 0x1000, 0, 0, 0, 0] {
        o.extend_from_slice(&v.to_le_bytes());
    }
    for c in px.chunks_exact(4) {
        match bytes {
            4 => o.extend_from_slice(&[c[2], c[1], c[0], c[3]]),
            3 => o.extend_from_slice(&[c[2], c[1], c[0]]),
            _ => {
                let v = ((c[0] as u16 >> 3) << 10) | ((c[1] as u16 >> 3) << 5) | (c[2] as u16 >> 3) | if c[3] >= 128 { 0x8000 } else { 0 };
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(o)
}
