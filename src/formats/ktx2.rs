//! KTX2 (Khronos Texture 2.0) : lecture de la première image (RGBA8 / RGB8 / BGRA8, BC7 ; sans compression, Zstandard ou zlib),
//! écriture sans perte (RGBA8) ou compressée en BC7, avec supercompression Zstandard.

use super::*;
use std::io::Read;

const MAGIC: [u8; 12] = [0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];

const VK_R8G8B8_UNORM: u32 = 23;
const VK_R8G8B8_SRGB: u32 = 29;
const VK_R8G8B8A8_UNORM: u32 = 37;
const VK_R8G8B8A8_SRGB: u32 = 43;
const VK_B8G8R8A8_UNORM: u32 = 44;
const VK_B8G8R8A8_SRGB: u32 = 50;
const VK_BC7_UNORM: u32 = 145;
const VK_BC7_SRGB: u32 = 146;

const SUPER_NONE: u32 = 0;
const SUPER_BASIS: u32 = 1;
const SUPER_ZSTD: u32 = 2;
const SUPER_ZLIB: u32 = 3;

fn le32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0)
}

fn le64(d: &[u8], o: usize) -> u64 {
    d.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).unwrap_or(0)
}

/// Taille maximale acceptée pour une image décompressée (protège contre un fichier qui annonce une taille démesurée).
const MAX_RAW: u64 = 1 << 30;

fn format_name(vk: u32) -> String {
    match vk {
        VK_R8G8B8_UNORM => "R8G8B8 UNORM".into(),
        VK_R8G8B8_SRGB => "R8G8B8 sRGB".into(),
        VK_R8G8B8A8_UNORM => "R8G8B8A8 UNORM".into(),
        VK_R8G8B8A8_SRGB => "R8G8B8A8 sRGB".into(),
        VK_B8G8R8A8_UNORM => "B8G8R8A8 UNORM".into(),
        VK_B8G8R8A8_SRGB => "B8G8R8A8 sRGB".into(),
        VK_BC7_UNORM => "BC7 UNORM".into(),
        VK_BC7_SRGB => "BC7 sRGB".into(),
        v => format!("VkFormat {v}"),
    }
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    if data.len() < 80 || data[..12] != MAGIC {
        bail!("signature KTX2 absente");
    }
    let vk = le32(data, 12);
    let (w, h, depth) = (le32(data, 20) as u64, le32(data, 24) as u64, le32(data, 28));
    let (layers, faces, levels, scheme) = (le32(data, 32), le32(data, 36), le32(data, 40), le32(data, 44));
    check_dims(w, h.max(1))?;
    if h == 0 {
        bail!("KTX2 : texture 1D non prise en charge");
    }
    let (w, h) = (w as usize, h as usize);
    if vk == 0 {
        bail!("KTX2 : format Basis Universal (VK_FORMAT_UNDEFINED) non pris en charge");
    }
    if scheme == SUPER_BASIS {
        bail!("KTX2 : supercompression BasisLZ non prise en charge");
    }
    // Index des niveaux : le premier est la pleine résolution.
    let (off, len, raw_len) = (le64(data, 80), le64(data, 88), le64(data, 96));
    if len == 0 || off.checked_add(len).is_none_or(|e| e > data.len() as u64) {
        bail!("KTX2 : données de niveau hors du fichier");
    }
    if raw_len > MAX_RAW {
        bail!("KTX2 : image décompressée trop grande");
    }
    let stored = &data[off as usize..(off + len) as usize];
    let raw: Vec<u8> = match scheme {
        SUPER_NONE => stored.to_vec(),
        SUPER_ZSTD => {
            let mut dec = ruzstd::decoding::StreamingDecoder::new(stored).map_err(|e| anyhow!("KTX2 : Zstandard : {e}"))?;
            let mut out = Vec::with_capacity((raw_len as usize).min(stored.len().saturating_mul(32)));
            dec.by_ref().take(MAX_RAW).read_to_end(&mut out).map_err(|e| anyhow!("KTX2 : Zstandard : {e}"))?;
            out
        }
        SUPER_ZLIB => {
            let mut out = Vec::with_capacity((raw_len as usize).min(stored.len().saturating_mul(32)));
            flate2::read::ZlibDecoder::new(stored).take(MAX_RAW).read_to_end(&mut out).map_err(|e| anyhow!("KTX2 : zlib : {e}"))?;
            out
        }
        s => bail!("KTX2 : supercompression {s} inconnue"),
    };
    let px = w * h;
    let need = |n: usize| -> Result<()> { if raw.len() < n { bail!("KTX2 : données insuffisantes ({} octets, {n} attendus)", raw.len()) } else { Ok(()) } };
    let (rgba, bpp): (Vec<u8>, u32) = match vk {
        VK_R8G8B8A8_UNORM | VK_R8G8B8A8_SRGB => {
            need(px * 4)?;
            (raw[..px * 4].to_vec(), 32)
        }
        VK_B8G8R8A8_UNORM | VK_B8G8R8A8_SRGB => {
            need(px * 4)?;
            (raw[..px * 4].chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect(), 32)
        }
        VK_R8G8B8_UNORM | VK_R8G8B8_SRGB => {
            need(px * 3)?;
            (raw[..px * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(), 24)
        }
        VK_BC7_UNORM | VK_BC7_SRGB => (dds::decode_bc7(&raw, w, h)?, 8),
        v => bail!("KTX2 : format {} non pris en charge", format_name(v)),
    };
    let mut pic = Picture::single(Format::Ktx2, w as u32, h as u32, Pixels::Rgba(rgba), None);
    pic.bpp = bpp;
    let s = pic.section("sec.format");
    s.add("p.compression", format!("{} — {}", format_name(vk), ["", "BasisLZ", "Zstandard", "zlib"].get(scheme as usize).filter(|n| !n.is_empty()).unwrap_or(&"—")));
    s.raw("Mipmaps", levels.max(1));
    if layers > 1 {
        s.raw("Layers (first shown)", layers);
    }
    if faces > 1 {
        s.raw("Cubemap", "first face shown");
    }
    if depth > 1 {
        s.raw("Depth (volume, first slice shown)", depth);
    }
    Ok(pic)
}

// ----------------------------------------------------------------------------- écriture

fn push32(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_le_bytes());
}

fn push64(o: &mut Vec<u8>, v: u64) {
    o.extend_from_slice(&v.to_le_bytes());
}

/// Descripteur de format de données (DFD) : RGBA8 sRGB, ou BC7 sRGB.
fn dfd(bc7: bool) -> Vec<u8> {
    let mut d = Vec::new();
    let samples = if bc7 { 1u32 } else { 4 };
    let block = 24 + 16 * samples;
    push32(&mut d, 4 + block); // taille totale
    push32(&mut d, 0); // fournisseur Khronos, type de descripteur de base
    push32(&mut d, 2 | (block << 16)); // version 2.0, taille du bloc
    // modèle de couleur (RGBSDA ou BC7), primaires BT.709, fonction de transfert sRGB, sans alpha prémultiplié
    push32(&mut d, (if bc7 { 134 } else { 1 }) | (1 << 8) | (2 << 16));
    push32(&mut d, if bc7 { 3 | (3 << 8) } else { 0 }); // dimensions du bloc de texels (moins 1)
    push32(&mut d, if bc7 { 16 } else { 4 }); // octets par bloc
    push32(&mut d, 0);
    if bc7 {
        push32(&mut d, 127 << 16); // 128 bits, canal 0
        push32(&mut d, 0);
        push32(&mut d, 0);
        push32(&mut d, u32::MAX);
    } else {
        // R, V, B, puis alpha (type 15, linéaire).
        for (i, ch) in [0u32, 1, 2, 15].into_iter().enumerate() {
            let lin = if ch == 15 { 1 << 28 } else { 0 };
            push32(&mut d, (i as u32 * 8) | (7 << 16) | (ch << 24) | lin);
            push32(&mut d, 0);
            push32(&mut d, 0);
            push32(&mut d, 255);
        }
    }
    d
}

pub fn encode(p: &Prepared, opts: &EncodeOpts) -> Result<Vec<u8>> {
    let f = p.first();
    let OutData::Rgba(px) = &f.data else { bail!("KTX2 : image RVB requise") };
    let (w, h) = (f.width as usize, f.height as usize);
    let rgba: std::borrow::Cow<[u8]> = if p.depth == Depth::Rgba32 {
        std::borrow::Cow::Borrowed(px)
    } else {
        // Sans alpha : opaque.
        std::borrow::Cow::Owned(px.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2], 255]).collect())
    };
    let (vk, raw) = if opts.ktx2_bc7 { (VK_BC7_SRGB, bc7_encode(&rgba, w, h)) } else { (VK_R8G8B8A8_SRGB, rgba.into_owned()) };
    let packed = ruzstd::encoding::compress_to_vec(&raw[..], ruzstd::encoding::CompressionLevel::Fastest);
    let dfd = dfd(opts.ktx2_bc7);
    let dfd_off = 80 + 24usize; // en-tête (80) + un niveau (24)
    let data_off = (dfd_off + dfd.len()).next_multiple_of(16);
    let mut o: Vec<u8> = Vec::with_capacity(data_off + packed.len());
    o.extend_from_slice(&MAGIC);
    for v in [vk, 1, w as u32, h as u32, 0, 0, 1, 1, SUPER_ZSTD] {
        push32(&mut o, v); // format, taille de type, largeur, hauteur, profondeur, couches, faces, niveaux, supercompression
    }
    for v in [dfd_off as u32, dfd.len() as u32, 0, 0] {
        push32(&mut o, v); // DFD, puis métadonnées clé/valeur (aucune)
    }
    push64(&mut o, 0); // données de supercompression globales (aucune)
    push64(&mut o, 0);
    push64(&mut o, data_off as u64);
    push64(&mut o, packed.len() as u64);
    push64(&mut o, raw.len() as u64);
    o.extend_from_slice(&dfd);
    o.resize(data_off, 0);
    o.extend_from_slice(&packed);
    Ok(o)
}

// ----------------------------------------------------------------------------- BC7 (mode 6, un seul sous-ensemble RGBA)

const WEIGHTS4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

struct BitWriter {
    bytes: [u8; 16],
    pos: usize,
}

impl BitWriter {
    fn put(&mut self, value: u32, bits: usize) {
        for i in 0..bits {
            if (value >> i) & 1 == 1 {
                self.bytes[self.pos / 8] |= 1 << (self.pos % 8);
            }
            self.pos += 1;
        }
    }
}

/// Point d'extrémité quantifié : 7 bits par canal plus le bit « p » commun.
#[derive(Clone, Copy)]
struct End {
    q: [u8; 4],
    p: u8,
}

impl End {
    fn value(&self) -> [i32; 4] {
        [0, 1, 2, 3].map(|c| ((self.q[c] as i32) << 1) | self.p as i32)
    }
}

/// Quantifie un point d'extrémité (flottant 0..255) en cherchant le bit « p » qui donne l'erreur la plus faible.
fn quantize(e: [f32; 4]) -> End {
    let mut best = (f32::MAX, End { q: [0; 4], p: 0 });
    for p in 0..2u8 {
        let mut q = [0u8; 4];
        let mut err = 0.0;
        for c in 0..4 {
            let v = ((e[c] - p as f32) / 2.0).round().clamp(0.0, 127.0);
            q[c] = v as u8;
            let d = e[c] - (v * 2.0 + p as f32);
            err += d * d;
        }
        if err < best.0 {
            best = (err, End { q, p });
        }
    }
    best.1
}

/// Indice (0..15) de la couleur interpolée la plus proche de chaque pixel, et erreur totale.
fn assign(px: &[[f32; 4]; 16], e0: &End, e1: &End) -> ([u8; 16], f32) {
    let (a, b) = (e0.value(), e1.value());
    let pal: [[i32; 4]; 16] = std::array::from_fn(|i| [0, 1, 2, 3].map(|c| ((64 - WEIGHTS4[i]) * a[c] + WEIGHTS4[i] * b[c] + 32) >> 6));
    let mut idx = [0u8; 16];
    let mut total = 0.0;
    for (k, p) in px.iter().enumerate() {
        let mut best = (f32::MAX, 0u8);
        for (i, c) in pal.iter().enumerate() {
            let e: f32 = (0..4).map(|ch| (p[ch] - c[ch] as f32).powi(2)).sum();
            if e < best.0 {
                best = (e, i as u8);
            }
        }
        idx[k] = best.1;
        total += best.0;
    }
    (idx, total)
}

fn encode_block(px: &[[f32; 4]; 16]) -> [u8; 16] {
    // Axe principal du nuage de couleurs (itération de puissance sur la covariance).
    let mean: [f32; 4] = std::array::from_fn(|c| px.iter().map(|p| p[c]).sum::<f32>() / 16.0);
    let mut cov = [[0f32; 4]; 4];
    for p in px {
        for i in 0..4 {
            for j in 0..4 {
                cov[i][j] += (p[i] - mean[i]) * (p[j] - mean[j]);
            }
        }
    }
    let mut axis = [1f32, 1.0, 1.0, 1.0];
    for _ in 0..8 {
        let mut n = [0f32; 4];
        for i in 0..4 {
            n[i] = (0..4).map(|j| cov[i][j] * axis[j]).sum();
        }
        let len = n.iter().map(|v| v * v).sum::<f32>().sqrt();
        if len < 1e-6 {
            break;
        }
        axis = n.map(|v| v / len);
    }
    let proj = |p: &[f32; 4]| -> f32 { (0..4).map(|c| (p[c] - mean[c]) * axis[c]).sum() };
    let (tmin, tmax) = px.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(proj(p)), hi.max(proj(p))));
    let at = |t: f32| -> [f32; 4] { std::array::from_fn(|c| (mean[c] + axis[c] * t).clamp(0.0, 255.0)) };
    let (mut e0, mut e1) = (quantize(at(tmin)), quantize(at(tmax)));
    let (mut idx, mut err) = assign(px, &e0, &e1);
    // Affinage par moindres carrés : meilleures extrémités pour les indices choisis.
    for _ in 0..2 {
        let (mut al, mut be, mut ga) = (0f32, 0f32, 0f32);
        let (mut r0, mut r1) = ([0f32; 4], [0f32; 4]);
        for (k, p) in px.iter().enumerate() {
            let a = WEIGHTS4[idx[k] as usize] as f32 / 64.0;
            al += (1.0 - a) * (1.0 - a);
            be += a * (1.0 - a);
            ga += a * a;
            for c in 0..4 {
                r0[c] += p[c] * (1.0 - a);
                r1[c] += p[c] * a;
            }
        }
        let det = al * ga - be * be;
        if det.abs() < 1e-4 {
            break;
        }
        let n0: [f32; 4] = std::array::from_fn(|c| ((ga * r0[c] - be * r1[c]) / det).clamp(0.0, 255.0));
        let n1: [f32; 4] = std::array::from_fn(|c| ((al * r1[c] - be * r0[c]) / det).clamp(0.0, 255.0));
        let (q0, q1) = (quantize(n0), quantize(n1));
        let (nidx, nerr) = assign(px, &q0, &q1);
        if nerr < err {
            (e0, e1, idx, err) = (q0, q1, nidx, nerr);
        } else {
            break;
        }
    }
    // Le premier indice n'a que 3 bits : son bit de poids fort doit valoir 0 (sinon on échange les extrémités).
    if idx[0] >= 8 {
        std::mem::swap(&mut e0, &mut e1);
        for i in idx.iter_mut() {
            *i = 15 - *i;
        }
    }
    let mut bw = BitWriter { bytes: [0; 16], pos: 0 };
    bw.put(1 << 6, 7); // mode 6 : six zéros puis un
    for c in 0..4 {
        bw.put(e0.q[c] as u32, 7);
        bw.put(e1.q[c] as u32, 7);
    }
    bw.put(e0.p as u32, 1);
    bw.put(e1.p as u32, 1);
    for (k, &i) in idx.iter().enumerate() {
        bw.put(i as u32, if k == 0 { 3 } else { 4 });
    }
    bw.bytes
}

/// Compresse une image RGBA en BC7 (blocs 4×4 ; les bords sont complétés en répétant le dernier pixel).
pub fn bc7_encode(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let mut out = Vec::with_capacity(bw * bh * 16);
    for by in 0..bh {
        for bx in 0..bw {
            let px: [[f32; 4]; 16] = std::array::from_fn(|i| {
                let (x, y) = ((bx * 4 + i % 4).min(w - 1), (by * 4 + i / 4).min(h - 1));
                let o = (y * w + x) * 4;
                [rgba[o] as f32, rgba[o + 1] as f32, rgba[o + 2] as f32, rgba[o + 3] as f32]
            });
            out.extend_from_slice(&encode_block(&px));
        }
    }
    out
}
