//! Paint.NET .PDN (format « PDN3 ») : lecture de l'image fusionnée, sans calques.
//!
//! Le format est sérialisé avec le BinaryFormatter de .NET (non documenté). Cette lecture n'analyse pas ce flux :
//! elle repère les blocs de pixels des calques (octet de version, taille de bloc, blocs gzip) et les superpose
//! (mode Normal, opacité pleine). Si le résultat diffère nettement de la miniature enregistrée dans l'en-tête XML
//! (calques masqués, opacité, modes de fusion), c'est la miniature qui est affichée.

use super::layered::blend_over;
use super::*;
use std::io::Read;

fn base64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for c in s.bytes() {
        if let Some(v) = val(c) {
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
    }
    out
}

fn attr<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(i) = xml[from..].find(&key) {
        let at = from + i;
        if at == 0 || xml.as_bytes()[at - 1].is_ascii_whitespace() {
            let s = at + key.len();
            let e = xml[s..].find('"')?;
            return Some(&xml[s..s + e]);
        }
        from = at + key.len();
    }
    None
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()))
}

/// Tente de lire un bloc de pixels à `p` ; renvoie (octets BGRA, position de fin).
fn read_block(d: &[u8], p: usize, len: usize) -> Option<(Vec<u8>, usize)> {
    let version = *d.get(p)?;
    if version > 1 {
        return None;
    }
    let chunk = be32(d, p + 1)? as usize;
    if !(1024..=(1 << 26)).contains(&chunk) {
        return None;
    }
    let count = len.div_ceil(chunk);
    let mut buf = vec![0u8; len];
    let mut seen = vec![false; count];
    let mut q = p + 5;
    for _ in 0..count {
        let (num, size) = (be32(d, q)? as usize, be32(d, q + 4)? as usize);
        q += 8;
        if num >= count || seen[num] {
            return None;
        }
        seen[num] = true;
        let raw = d.get(q..q.checked_add(size)?)?;
        q += size;
        let off = num * chunk;
        let want = chunk.min(len - off);
        if version == 0 {
            if raw.len() < 2 || raw[0] != 0x1F || raw[1] != 0x8B {
                return None;
            }
            let mut v = Vec::with_capacity(want);
            flate2::read::GzDecoder::new(raw).read_to_end(&mut v).ok()?;
            if v.len() < want {
                return None;
            }
            buf[off..off + want].copy_from_slice(&v[..want]);
        } else {
            if raw.len() < want {
                return None;
            }
            buf[off..off + want].copy_from_slice(&raw[..want]);
        }
    }
    Some((buf, q))
}

fn find_first_block(d: &[u8], from: usize, len: usize) -> Option<(Vec<u8>, usize)> {
    for p in from..d.len().saturating_sub(16) {
        if d[p] <= 1 && (d[p] == 1 || d.get(p + 13..p + 15) == Some(&[0x1F, 0x8B])) {
            if let Some(r) = read_block(d, p, len) {
                return Some(r);
            }
        }
    }
    None
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    if r.bytes(4)? != b"PDN3" {
        bail!("signature PDN3 absente");
    }
    let hl = r.bytes(3)?;
    let hlen = hl[0] as usize | (hl[1] as usize) << 8 | (hl[2] as usize) << 16;
    let xml = String::from_utf8_lossy(r.bytes(hlen)?).to_string();
    let marker = r.bytes(2)?;
    if marker != [0x00, 0x01] {
        bail!("PDN : marqueur de données inattendu");
    }
    let body = r.pos;
    let w: usize = attr(&xml, "width").and_then(|v| v.parse().ok()).ok_or_else(|| anyhow!("PDN : largeur absente de l'en-tête"))?;
    let h: usize = attr(&xml, "height").and_then(|v| v.parse().ok()).ok_or_else(|| anyhow!("PDN : hauteur absente de l'en-tête"))?;
    let nlayers: usize = attr(&xml, "layers").and_then(|v| v.parse().ok()).unwrap_or(1);
    check_dims(w as u64, h as u64)?;
    let thumb: Option<(u32, u32, Vec<u8>)> = attr(&xml, "png").and_then(|b| {
        let img = image::load_from_memory_with_format(&base64(b), image::ImageFormat::Png).ok()?;
        let rgba = img.to_rgba8();
        Some((rgba.width(), rgba.height(), rgba.into_raw()))
    });

    // Calques : blocs de pixels consécutifs (BGRA).
    let len = w * h * 4;
    let mut layers: Vec<Vec<u8>> = Vec::new();
    if let Some((first, mut end)) = find_first_block(data, body, len) {
        layers.push(first);
        while layers.len() < nlayers {
            match read_block(data, end, len).or_else(|| find_first_block(data, end, len)) {
                Some((b, e)) => {
                    layers.push(b);
                    end = e;
                }
                None => break,
            }
        }
    }
    let mut canvas = vec![0u8; len];
    for l in &layers {
        let mut rgba = l.clone();
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        blend_over(&mut canvas, w, h, &rgba, w, h, 0, 0, 1.0);
    }
    // Contrôle par la miniature : écart moyen après réduction.
    let mut use_layers = layers.len() == nlayers && !layers.is_empty();
    if let (true, Some((tw, th, t))) = (use_layers && nlayers > 1, &thumb) {
        let (tw, th) = (*tw as usize, *th as usize);
        let (mut diff, mut n) = (0u64, 0u64);
        for y in 0..th {
            for x in 0..tw {
                let (sx, sy) = (x * w / tw, y * h / th);
                let (a, b) = (&canvas[(sy * w + sx) * 4..(sy * w + sx) * 4 + 3], &t[(y * tw + x) * 4..(y * tw + x) * 4 + 3]);
                for c in 0..3 {
                    diff += (a[c] as i32 - b[c] as i32).unsigned_abs() as u64;
                    n += 1;
                }
            }
        }
        if n > 0 && diff as f64 / n as f64 > 24.0 {
            use_layers = false;
        }
    }
    let (source, mut pic) = if use_layers {
        ("layers", Picture::single(Format::Pdn, w as u32, h as u32, Pixels::Rgba(canvas), None))
    } else if let Some((tw, th, t)) = thumb {
        ("thumbnail", Picture::single(Format::Pdn, tw, th, Pixels::Rgba(t), None))
    } else {
        bail!("PDN : pixels des calques introuvables et pas de miniature");
    };
    pic.bpp = 32;
    let s = pic.section("sec.format");
    s.raw("Source", if source == "layers" { "layers (composite)" } else { "thumbnail (header)" });
    s.add("p.dimensions", format!("{w} × {h}"));
    s.add("p.layers", nlayers);
    if let Some(v) = attr(&xml, "savedWith").or_else(|| attr(&xml, "savedWithVersion")) {
        s.raw("Saved with", v);
    }
    Ok(pic)
}
