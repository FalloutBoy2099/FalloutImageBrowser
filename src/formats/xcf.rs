//! GIMP XCF : lecture de l'image fusionnée en superposant les calques visibles (mode Normal).
//! https://developer.gimp.org/core/standards/xcf/ — un XCF ne contient pas d'image fusionnée.

use super::layered::blend_over;
use super::*;
use std::io::Read;

struct Ctx<'a> {
    d: &'a [u8],
    ptr8: bool,
    compression: u8,
    bpc: usize,
    linear: bool,
}

impl Ctx<'_> {
    fn ptr(&self, r: &mut Rd) -> Result<usize> {
        if self.ptr8 {
            let hi = r.u32be()? as u64;
            Ok(((hi << 32) | r.u32be()? as u64) as usize)
        } else {
            Ok(r.u32be()? as usize)
        }
    }
}

fn read_string(r: &mut Rd) -> Result<String> {
    let n = r.u32be()? as usize;
    if n == 0 {
        return Ok(String::new());
    }
    let b = r.bytes(n)?;
    Ok(String::from_utf8_lossy(&b[..n - 1]).to_string())
}

/// Décode un flux RLE de GIMP pour `bpp` plans consécutifs de `n` octets ; renvoie les plans entrelacés.
fn rle_tile(src: &[u8], n: usize, bpp: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; n * bpp];
    let mut i = 0usize;
    for k in 0..bpp {
        let mut o = 0usize;
        while o < n {
            let op = *src.get(i).ok_or_else(|| anyhow!("tuile RLE tronquée"))?;
            i += 1;
            let (len, literal) = match op {
                0..=126 => (op as usize + 1, false),
                127 => {
                    let l = u16::from_be_bytes([*src.get(i).unwrap_or(&0), *src.get(i + 1).unwrap_or(&0)]) as usize;
                    i += 2;
                    (l, false)
                }
                128 => {
                    let l = u16::from_be_bytes([*src.get(i).unwrap_or(&0), *src.get(i + 1).unwrap_or(&0)]) as usize;
                    i += 2;
                    (l, true)
                }
                _ => (256 - op as usize, true),
            };
            let len = len.min(n - o);
            if literal {
                let s = src.get(i..i + len).ok_or_else(|| anyhow!("tuile RLE tronquée"))?;
                for (j, v) in s.iter().enumerate() {
                    out[(o + j) * bpp + k] = *v;
                }
                i += len;
            } else {
                let v = *src.get(i).ok_or_else(|| anyhow!("tuile RLE tronquée"))?;
                i += 1;
                for j in 0..len {
                    out[(o + j) * bpp + k] = v;
                }
            }
            o += len;
        }
    }
    Ok(out)
}

/// Lit une hiérarchie (niveau 0 seulement) et renvoie (octets entrelacés, bpp).
fn read_hierarchy(c: &Ctx, off: usize, w: usize, h: usize) -> Result<(Vec<u8>, usize)> {
    let mut r = Rd::at(c.d, off);
    let (_hw, _hh, bpp) = (r.u32be()?, r.u32be()?, r.u32be()? as usize);
    if bpp == 0 || bpp > 16 {
        bail!("XCF : octets par pixel invalides ({bpp})");
    }
    let lptr = c.ptr(&mut r)?;
    let mut r = Rd::at(c.d, lptr);
    let (lw, lh) = (r.u32be()? as usize, r.u32be()? as usize);
    if lw != w || lh != h {
        bail!("XCF : dimensions de niveau incohérentes");
    }
    let (cols, rows) = (w.div_ceil(64), h.div_ceil(64));
    let mut buf = vec![0u8; w * h * bpp];
    for t in 0..cols * rows {
        let p = c.ptr(&mut r)?;
        if p == 0 {
            bail!("XCF : tuile manquante");
        }
        let (tx, ty) = (t % cols, t / cols);
        let (tw, th) = (64.min(w - tx * 64), 64.min(h - ty * 64));
        let n = tw * th;
        let src = c.d.get(p..).ok_or_else(|| anyhow!("XCF tronqué"))?;
        let tile: Vec<u8> = match c.compression {
            0 => src.get(..n * bpp).ok_or_else(|| anyhow!("tuile tronquée"))?.to_vec(),
            1 => rle_tile(src, n, bpp)?,
            2 => {
                let mut v = Vec::with_capacity(n * bpp);
                flate2::read::ZlibDecoder::new(src).take((n * bpp) as u64).read_to_end(&mut v).map_err(|e| anyhow!("XCF zlib : {e}"))?;
                if v.len() < n * bpp {
                    bail!("tuile zlib incomplète");
                }
                v
            }
            m => bail!("compression XCF inconnue ({m})"),
        };
        for y in 0..th {
            let dst = ((ty * 64 + y) * w + tx * 64) * bpp;
            buf[dst..dst + tw * bpp].copy_from_slice(&tile[y * tw * bpp..(y + 1) * tw * bpp]);
        }
    }
    Ok((buf, bpp))
}

fn srgb_from_linear(v: u8) -> u8 {
    let l = v as f32 / 255.0;
    let s = if l <= 0.003_130_8 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

struct Layer {
    w: usize,
    h: usize,
    kind: u32,
    visible: bool,
    group: bool,
    opacity: f32,
    off: (i64, i64),
    apply_mask: bool,
    hptr: usize,
    mptr: usize,
}

fn read_layer(c: &Ctx, off: usize) -> Result<Layer> {
    let mut r = Rd::at(c.d, off);
    let (w, h, kind) = (r.u32be()? as usize, r.u32be()? as usize, r.u32be()?);
    let _name = read_string(&mut r)?;
    let mut l = Layer { w, h, kind, visible: true, group: false, opacity: 1.0, off: (0, 0), apply_mask: true, hptr: 0, mptr: 0 };
    loop {
        let (ty, len) = (r.u32be()?, r.u32be()? as usize);
        if ty == 0 {
            break;
        }
        let p = r.bytes(len)?;
        let u32at = |i: usize| p.get(i..i + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
        match ty {
            6 => l.opacity = u32at(0).unwrap_or(255) as f32 / 255.0,
            33 => l.opacity = u32at(0).map(f32::from_bits).unwrap_or(1.0),
            8 => l.visible = u32at(0).unwrap_or(1) != 0,
            15 => l.off = (u32at(0).unwrap_or(0) as i32 as i64, u32at(4).unwrap_or(0) as i32 as i64),
            11 => l.apply_mask = u32at(0).unwrap_or(1) != 0,
            29 => l.group = true,
            _ => {}
        }
    }
    l.hptr = c.ptr(&mut r)?;
    l.mptr = c.ptr(&mut r)?;
    Ok(l)
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    if r.bytes(9)? != b"gimp xcf " {
        bail!("signature XCF absente");
    }
    let tag = r.bytes(4)?;
    let version: u32 = if tag == b"file" { 0 } else { std::str::from_utf8(&tag[1..]).ok().and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("version XCF inconnue"))? };
    r.skip(1)?;
    let (w, h, base) = (r.u32be()? as usize, r.u32be()? as usize, r.u32be()?);
    check_dims(w as u64, h as u64)?;
    let precision = if version >= 4 { r.u32be()? } else { 150 };
    let (bpc, linear) = if version < 7 {
        (1, false)
    } else {
        match precision {
            100 => (1, true),
            150 => (1, false),
            200 => (2, true),
            250 => (2, false),
            p => bail!("XCF : précision {p} non prise en charge (entiers 8 et 16 bits uniquement)"),
        }
    };
    let ptr8 = version >= 11;
    let mut c = Ctx { d: data, ptr8, compression: 1, bpc, linear };
    let mut colormap: Vec<[u8; 3]> = Vec::new();
    loop {
        let (ty, len) = (r.u32be()?, r.u32be()? as usize);
        if ty == 0 {
            break;
        }
        let p = r.bytes(len)?;
        match ty {
            1 if p.len() >= 4 => {
                let n = u32::from_be_bytes(p[..4].try_into().unwrap()) as usize;
                colormap = p[4..].chunks_exact(3).take(n.min(256)).map(|c| [c[0], c[1], c[2]]).collect();
            }
            17 => c.compression = *p.first().unwrap_or(&1),
            _ => {}
        }
    }
    let mut layer_ptrs = Vec::new();
    loop {
        let p = c.ptr(&mut r)?;
        if p == 0 {
            break;
        }
        layer_ptrs.push(p);
        if layer_ptrs.len() > 100_000 {
            bail!("XCF : trop de calques");
        }
    }
    let mut canvas = vec![0u8; w * h * 4];
    let (mut used, mut groups) = (0usize, 0usize);
    // Les calques sont listés du plus haut au plus bas : on dessine en sens inverse.
    for &lp in layer_ptrs.iter().rev() {
        let l = read_layer(&c, lp)?;
        if l.group {
            groups += 1;
            continue;
        }
        if !l.visible || l.w == 0 || l.h == 0 {
            continue;
        }
        check_dims(l.w as u64, l.h as u64)?;
        let (buf, bpp) = read_hierarchy(&c, l.hptr, l.w, l.h)?;
        let (cc, has_alpha) = match l.kind {
            0 => (3, false),
            1 => (3, true),
            2 => (1, false),
            3 => (1, true),
            4 => (1, false),
            5 => (1, true),
            k => bail!("XCF : type de calque inconnu ({k})"),
        };
        let nch = cc + has_alpha as usize;
        if bpp != nch * c.bpc {
            bail!("XCF : {bpp} octets par pixel, {} attendus", nch * c.bpc);
        }
        let mask = if l.mptr != 0 && l.apply_mask {
            let mut mr = Rd::at(c.d, l.mptr);
            let (mw, mh) = (mr.u32be()? as usize, mr.u32be()? as usize);
            let _ = read_string(&mut mr)?;
            loop {
                let (ty, len) = (mr.u32be()?, mr.u32be()? as usize);
                if ty == 0 {
                    break;
                }
                mr.skip(len)?;
            }
            let mh_ptr = c.ptr(&mut mr)?;
            if mw == l.w && mh == l.h { read_hierarchy(&c, mh_ptr, mw, mh).ok() } else { None }
        } else {
            None
        };
        let mut px = vec![0u8; l.w * l.h * 4];
        let ch8 = |i: usize, ch: usize| -> u8 { buf[i * bpp + ch * c.bpc] };
        for i in 0..l.w * l.h {
            let o = i * 4;
            let col = |v: u8| if c.linear { srgb_from_linear(v) } else { v };
            match l.kind {
                0 | 1 => {
                    px[o] = col(ch8(i, 0));
                    px[o + 1] = col(ch8(i, 1));
                    px[o + 2] = col(ch8(i, 2));
                }
                2 | 3 => {
                    let v = col(ch8(i, 0));
                    px[o..o + 3].fill(v);
                }
                _ => {
                    let e = colormap.get(ch8(i, 0) as usize).copied().unwrap_or([0, 0, 0]);
                    px[o..o + 3].copy_from_slice(&e);
                }
            }
            let mut a = if has_alpha { ch8(i, cc) as u32 } else { 255 };
            if l.kind >= 4 && has_alpha {
                a = if a >= 128 { 255 } else { 0 };
            }
            if let Some((mb, mbpp)) = &mask {
                a = a * mb[i * mbpp] as u32 / 255;
            }
            px[o + 3] = a as u8;
        }
        blend_over(&mut canvas, w, h, &px, l.w, l.h, l.off.0, l.off.1, l.opacity);
        used += 1;
    }
    let mut pic = Picture::single(Format::Xcf, w as u32, h as u32, Pixels::Rgba(canvas), None);
    pic.bpp = 32;
    let s = pic.section("sec.format");
    s.add("p.version", if version == 0 { "0".to_string() } else { version.to_string() });
    s.raw("Base type", match base { 0 => "RGB", 1 => "Grayscale", _ => "Indexed" });
    s.add("p.precision", if version >= 7 { precision.to_string() } else { "8".to_string() });
    s.add("p.compression", match c.compression { 0 => "p.none", 1 => "RLE", _ => "zlib" });
    s.add("p.layers", layer_ptrs.len());
    s.raw("Visible layers composed", used);
    if groups > 0 {
        s.raw("Layer groups", groups);
    }
    Ok(pic)
}
