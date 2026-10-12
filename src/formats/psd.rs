//! Adobe Photoshop PSD / PSB : lecture de l'image fusionnée (section « Image Data »), sans calques.

use super::*;
use std::io::Read;

fn u64be(r: &mut Rd) -> Result<u64> {
    let hi = r.u32be()? as u64;
    Ok((hi << 32) | r.u32be()? as u64)
}

fn mode_name(m: u16) -> &'static str {
    match m {
        0 => "Bitmap",
        1 => "Grayscale",
        2 => "Indexed",
        3 => "RGB",
        4 => "CMYK",
        7 => "Multichannel",
        8 => "Duotone",
        9 => "Lab",
        _ => "?",
    }
}

fn srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

fn lab_to_rgb(l: f32, a: f32, b: f32) -> [u8; 3] {
    let fy = (l + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let inv = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.9642 * inv(fx), inv(fy), 0.8249 * inv(fz));
    [srgb(3.133_856 * x - 1.6168667 * y - 0.4906146 * z), srgb(-0.9787684 * x + 1.9161415 * y + 0.0334540 * z), srgb(0.0719453 * x - 0.2289914 * y + 1.4052427 * z)]
}

/// Ressources d'image : (résolution horizontale en ppp, index transparent).
fn resources(res: &[u8]) -> (Option<f64>, Option<u16>) {
    let (mut dpi, mut transp) = (None, None);
    let mut p = 0usize;
    while p + 12 <= res.len() && &res[p..p + 4] == b"8BIM" {
        let id = u16::from_be_bytes([res[p + 4], res[p + 5]]);
        let name_len = res[p + 6] as usize;
        let name_total = (1 + name_len + 1) & !1;
        let sp = p + 6 + name_total;
        if sp + 4 > res.len() {
            break;
        }
        let size = u32::from_be_bytes(res[sp..sp + 4].try_into().unwrap()) as usize;
        let data = res.get(sp + 4..sp + 4 + size).unwrap_or(&[]);
        match id {
            0x03ED if data.len() >= 4 => dpi = Some(u32::from_be_bytes(data[..4].try_into().unwrap()) as f64 / 65536.0),
            0x0417 if data.len() >= 2 => transp = Some(u16::from_be_bytes([data[0], data[1]])),
            _ => {}
        }
        p = sp + 4 + ((size + 1) & !1);
    }
    (dpi, transp)
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    if r.bytes(4)? != b"8BPS" {
        bail!("signature PSD absente");
    }
    let version = r.u16be()?;
    if version != 1 && version != 2 {
        bail!("version PSD inconnue ({version})");
    }
    r.skip(6)?;
    let channels = r.u16be()? as usize;
    let h = r.u32be()? as usize;
    let w = r.u32be()? as usize;
    let depth = r.u16be()? as usize;
    let mode = r.u16be()?;
    check_dims(w as u64, h as u64)?;
    if !matches!(depth, 1 | 8 | 16 | 32) || channels == 0 || channels > 56 {
        bail!("PSD : profondeur ou nombre de canaux invalide ({depth} bits, {channels} canaux)");
    }
    let cm_len = r.u32be()? as usize;
    let cmode = r.bytes(cm_len)?;
    let ir_len = r.u32be()? as usize;
    let res = r.bytes(ir_len)?;
    let lm_len = if version == 2 { u64be(&mut r)? as usize } else { r.u32be()? as usize };
    let lm = r.bytes(lm_len)?;
    // Nombre de calques ; négatif : le premier canal alpha contient la transparence de l'image fusionnée.
    let (mut layers, mut merged_alpha) = (0i32, false);
    {
        let mut lr = Rd::new(lm);
        let info_len = if version == 2 { u64be(&mut lr).unwrap_or(0) } else { lr.u32be().unwrap_or(0) as u64 };
        if info_len > 0
            && let Ok(c) = lr.u16be()
        {
            let c = c as i16 as i32;
            layers = c.abs();
            merged_alpha = c < 0;
        }
    }
    let (dpi, transp_index) = resources(res);
    let compression = r.u16be()?;
    let bps = if depth == 1 { 1 } else { depth / 8 };
    let row_bytes = if depth == 1 { w.div_ceil(8) } else { w * bps };
    let plane = row_bytes * h;
    let total = plane.checked_mul(channels).filter(|t| *t < 3_000_000_000).ok_or_else(|| anyhow!("PSD trop grand"))?;
    let rest = &data[r.pos.min(data.len())..];
    let mut planar: Vec<u8>;
    match compression {
        0 => {
            planar = rest.get(..total).ok_or_else(|| anyhow!("données PSD tronquées"))?.to_vec();
        }
        1 => {
            let n = channels * h;
            let cw = if version == 2 { 4 } else { 2 };
            let table = rest.get(..n * cw).ok_or_else(|| anyhow!("table RLE tronquée"))?;
            let mut p = n * cw;
            planar = vec![0u8; total];
            for (row, ch) in table.chunks_exact(cw).enumerate() {
                let len = if cw == 2 { u16::from_be_bytes([ch[0], ch[1]]) as usize } else { u32::from_be_bytes(ch.try_into().unwrap()) as usize };
                let src = rest.get(p..p + len).ok_or_else(|| anyhow!("données RLE tronquées"))?;
                p += len;
                let dst = &mut planar[row * row_bytes..(row + 1) * row_bytes];
                let (mut i, mut o) = (0usize, 0usize);
                while i < src.len() && o < row_bytes {
                    let c = src[i] as i8;
                    i += 1;
                    if c >= 0 {
                        let n = (c as usize + 1).min(row_bytes - o).min(src.len() - i);
                        dst[o..o + n].copy_from_slice(&src[i..i + n]);
                        i += c as usize + 1;
                        o += n;
                    } else if c != -128 {
                        let n = (1 - c as isize) as usize;
                        let v = *src.get(i).unwrap_or(&0);
                        i += 1;
                        let n = n.min(row_bytes - o);
                        dst[o..o + n].fill(v);
                        o += n;
                    }
                }
            }
        }
        2 | 3 => {
            planar = Vec::with_capacity(total);
            flate2::read::ZlibDecoder::new(rest).read_to_end(&mut planar).map_err(|e| anyhow!("PSD zip : {e}"))?;
            if planar.len() < total {
                bail!("données PSD insuffisantes");
            }
            planar.truncate(total);
            if compression == 3 {
                match depth {
                    8 => {
                        for row in planar.chunks_exact_mut(row_bytes) {
                            for x in 1..row.len() {
                                row[x] = row[x].wrapping_add(row[x - 1]);
                            }
                        }
                    }
                    16 => {
                        for row in planar.chunks_exact_mut(row_bytes) {
                            let mut prev = 0u16;
                            for px in row.chunks_exact_mut(2) {
                                prev = prev.wrapping_add(u16::from_be_bytes([px[0], px[1]]));
                                px.copy_from_slice(&prev.to_be_bytes());
                            }
                        }
                    }
                    _ => bail!("PSD : prédiction non prise en charge pour {depth} bits"),
                }
            }
        }
        c => bail!("compression PSD inconnue ({c})"),
    }

    // Échelle des échantillons 16 bits (Photoshop : 0..32768 ; sinon 0..65535).
    let scale16: f32 = if depth == 16 {
        let max = planar.chunks_exact(2).map(|p| u16::from_be_bytes([p[0], p[1]])).max().unwrap_or(0);
        if max <= 32768 { 32768.0 } else { 65535.0 }
    } else {
        1.0
    };
    // Échantillon d'un canal en 0..=255.
    let sample = |ch: usize, x: usize, y: usize| -> u8 {
        let base = ch * plane + y * row_bytes;
        match depth {
            8 => planar[base + x],
            16 => {
                let v = u16::from_be_bytes([planar[base + x * 2], planar[base + x * 2 + 1]]) as f32;
                (v / scale16 * 255.0 + 0.5).min(255.0) as u8
            }
            32 => {
                let o = base + x * 4;
                srgb(f32::from_be_bytes([planar[o], planar[o + 1], planar[o + 2], planar[o + 3]]))
            }
            _ => ((planar[base + x / 8] >> (7 - x % 8)) & 1) * 255, // 1 bit
        }
    };
    let mut pic;
    if mode == 2 && depth == 8 && cmode.len() >= 768 {
        let mut pal = Palette::empty();
        for i in 0..256 {
            pal.colors[i] = [cmode[i], cmode[256 + i], cmode[512 + i], 255];
            pal.used[i] = true;
        }
        if let Some(t) = transp_index
            && t < 256
        {
            pal.colors[t as usize][3] = 0;
        }
        let mut idx = vec![0u8; w * h];
        for y in 0..h {
            idx[y * w..(y + 1) * w].copy_from_slice(&planar[y * row_bytes..y * row_bytes + w]);
        }
        pic = Picture::single(Format::Psd, w as u32, h as u32, Pixels::Indexed(idx), Some(pal));
    } else {
        let mut out = vec![255u8; w * h * 4];
        let alpha = |nch: usize| if merged_alpha && channels > nch { Some(nch) } else { None };
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                match mode {
                    3 if channels >= 3 => {
                        out[o] = sample(0, x, y);
                        out[o + 1] = sample(1, x, y);
                        out[o + 2] = sample(2, x, y);
                        if let Some(a) = alpha(3) {
                            out[o + 3] = sample(a, x, y);
                        }
                    }
                    0 => {
                        // Bitmap : 1 = noir.
                        let v = 255 - sample(0, x, y);
                        out[o..o + 3].fill(v);
                    }
                    1 | 2 | 7 | 8 => {
                        let v = sample(0, x, y);
                        out[o..o + 3].fill(v);
                        if let Some(a) = alpha(1) {
                            out[o + 3] = sample(a, x, y);
                        }
                    }
                    4 if channels >= 4 => {
                        // CMJN stocké inversé (255 = pas d'encre).
                        let k = sample(3, x, y) as u32;
                        for c in 0..3 {
                            out[o + c] = (sample(c, x, y) as u32 * k / 255) as u8;
                        }
                        if let Some(a) = alpha(4) {
                            out[o + 3] = sample(a, x, y);
                        }
                    }
                    9 if channels >= 3 => {
                        let rgb = lab_to_rgb(sample(0, x, y) as f32 / 255.0 * 100.0, sample(1, x, y) as f32 - 128.0, sample(2, x, y) as f32 - 128.0);
                        out[o..o + 3].copy_from_slice(&rgb);
                    }
                    _ => bail!("PSD : mode couleur non pris en charge ({}, {channels} canaux)", mode_name(mode)),
                }
            }
        }
        pic = Picture::single(Format::Psd, w as u32, h as u32, Pixels::Rgba(out), None);
    }
    pic.bpp = (channels * depth) as u32;
    let s = pic.section("sec.format");
    s.raw("Type", if version == 2 { "PSB (large document)" } else { "PSD" });
    s.add("p.channels", channels);
    s.add("p.precision", depth);
    s.raw("Color mode", mode_name(mode));
    s.add(
        "p.compression",
        match compression {
            0 => "p.none",
            1 => "RLE (PackBits)",
            2 => "ZIP",
            _ => "ZIP + prediction",
        },
    );
    s.add("p.layers", layers);
    if let Some(d) = dpi {
        s.add("p.dpi", format!("{d:.0}"));
    }
    Ok(pic)
}
