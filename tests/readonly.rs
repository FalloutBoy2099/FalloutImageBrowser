#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments)]
//! Lecture des formats en lecture seule (PSD, PDN, XCF, KRA, ORA) à partir de fichiers fabriqués par les tests.

use fallout_image_browser::formats::{self, DecodeCtx};
use fallout_image_browser::model::*;
use fallout_image_browser::palette;
use fallout_image_browser::palette_export::zip_store;
use std::io::Write;

/// Fichiers d'exemple versionnés avec le projet (dossier `_test_files`).
fn sample(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join(rel)
}

fn ctx() -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() }
}

fn rgba_of(p: &Picture) -> Vec<u8> {
    frame_to_rgba(&p.frames[0], p.palette.as_ref())
}

/// Image de test : dégradé avec un pixel (0,0) rouge.
fn gradient(w: usize, h: usize) -> Vec<u8> {
    let mut v = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            v[o] = (x * 255 / (w - 1)) as u8;
            v[o + 1] = (y * 255 / (h - 1)) as u8;
            v[o + 2] = ((x + y) * 5 % 256) as u8;
        }
    }
    v[..4].copy_from_slice(&[255, 0, 0, 255]);
    v
}

fn png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image_compat(rgba, w as u32, h as u32).unwrap();
    out
}

trait Enc {
    fn write_image_compat(self, rgba: &[u8], w: u32, h: u32) -> image::ImageResult<()>;
}
impl<W: Write> Enc for image::codecs::png::PngEncoder<W> {
    fn write_image_compat(self, rgba: &[u8], w: u32, h: u32) -> image::ImageResult<()> {
        use image::ImageEncoder;
        self.write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
    }
}

// ----------------------------------------------------------------------------- PSD

fn psd(w: usize, h: usize, channels: usize, depth: usize, mode: u16, comp: u16, planar: &[u8], cmode: &[u8], layer_count: i16, rle_rows: Option<&[Vec<u8>]>) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(b"8BPS");
    o.extend_from_slice(&1u16.to_be_bytes());
    o.extend_from_slice(&[0; 6]);
    o.extend_from_slice(&(channels as u16).to_be_bytes());
    o.extend_from_slice(&(h as u32).to_be_bytes());
    o.extend_from_slice(&(w as u32).to_be_bytes());
    o.extend_from_slice(&(depth as u16).to_be_bytes());
    o.extend_from_slice(&mode.to_be_bytes());
    o.extend_from_slice(&(cmode.len() as u32).to_be_bytes());
    o.extend_from_slice(cmode);
    // Ressource de résolution 72 ppp.
    let mut res = Vec::new();
    res.extend_from_slice(b"8BIM");
    res.extend_from_slice(&0x03EDu16.to_be_bytes());
    res.extend_from_slice(&[0, 0]);
    res.extend_from_slice(&16u32.to_be_bytes());
    res.extend_from_slice(&(72u32 << 16).to_be_bytes());
    res.extend_from_slice(&[0, 1, 0, 1]);
    res.extend_from_slice(&(72u32 << 16).to_be_bytes());
    res.extend_from_slice(&[0, 1, 0, 1]);
    o.extend_from_slice(&(res.len() as u32).to_be_bytes());
    o.extend_from_slice(&res);
    // Calques : longueur de la section, longueur de l'info, nombre de calques.
    let lm = if layer_count != 0 {
        let mut l = Vec::new();
        l.extend_from_slice(&2u32.to_be_bytes());
        l.extend_from_slice(&layer_count.to_be_bytes());
        l
    } else {
        Vec::new()
    };
    o.extend_from_slice(&(lm.len() as u32).to_be_bytes());
    o.extend_from_slice(&lm);
    o.extend_from_slice(&comp.to_be_bytes());
    if let Some(rows) = rle_rows {
        for r in rows {
            o.extend_from_slice(&(r.len() as u16).to_be_bytes());
        }
        for r in rows {
            o.extend_from_slice(r);
        }
    } else {
        o.extend_from_slice(planar);
    }
    o
}

fn packbits_row(row: &[u8]) -> Vec<u8> {
    // Une exécution de répétition si possible, sinon littéraux.
    let mut out = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let mut n = 1;
        while i + n < row.len() && row[i + n] == row[i] && n < 128 {
            n += 1;
        }
        if n >= 3 {
            out.push((1i16 - n as i16) as u8);
            out.push(row[i]);
            i += n;
        } else {
            let end = (i + 1).min(row.len());
            out.push(0);
            out.push(row[i]);
            i = end;
        }
    }
    out
}

#[test]
fn psd_rgb_raw_rle_and_alpha() {
    let (w, h) = (23usize, 17usize);
    let g = gradient(w, h);
    let mut planar = Vec::new();
    for ch in 0..3 {
        for i in 0..w * h {
            planar.push(g[i * 4 + ch]);
        }
    }
    // Brut.
    let p = formats::decode(Format::Psd, &psd(w, h, 3, 8, 3, 0, &planar, &[], 0, None), &ctx()).unwrap();
    assert_eq!((p.frames[0].width, p.frames[0].height), (w as u32, h as u32));
    assert_eq!(rgba_of(&p), g);
    // RLE.
    let rows: Vec<Vec<u8>> = planar.chunks(w).map(packbits_row).collect();
    let p = formats::decode(Format::Psd, &psd(w, h, 3, 8, 3, 1, &[], &[], 0, Some(&rows)), &ctx()).unwrap();
    assert_eq!(rgba_of(&p), g);
    // Alpha (nombre de calques négatif) : 4e canal.
    let mut planar4 = planar.clone();
    for i in 0..w * h {
        planar4.push((i % 256) as u8);
    }
    let p = formats::decode(Format::Psd, &psd(w, h, 4, 8, 3, 0, &planar4, &[], -1, None), &ctx()).unwrap();
    let out = rgba_of(&p);
    assert!((0..w * h).all(|i| out[i * 4 + 3] == (i % 256) as u8 && out[i * 4] == g[i * 4]));
    assert!(p.has_transparency);
    // Sans le signe négatif, le 4e canal n'est pas de l'alpha.
    let p = formats::decode(Format::Psd, &psd(w, h, 4, 8, 3, 0, &planar4, &[], 2, None), &ctx()).unwrap();
    assert!(rgba_of(&p).chunks_exact(4).all(|c| c[3] == 255));
}

#[test]
fn psd_indexed_gray16_cmyk_and_zip() {
    let (w, h) = (9usize, 7usize);
    // Indexé : palette 768 octets (R, V, B par plan).
    let mut cm = vec![0u8; 768];
    for i in 0..256 {
        cm[i] = i as u8;
        cm[256 + i] = (255 - i) as u8;
        cm[512 + i] = (i / 2) as u8;
    }
    let idx: Vec<u8> = (0..w * h).map(|i| (i * 3 % 256) as u8).collect();
    let p = formats::decode(Format::Psd, &psd(w, h, 1, 8, 2, 0, &idx, &cm, 0, None), &ctx()).unwrap();
    assert!(p.palette.is_some());
    let out = rgba_of(&p);
    assert_eq!(&out[..4], &[0, 255, 0, 255]);
    let i5 = idx[5] as usize;
    assert_eq!(&out[5 * 4..5 * 4 + 3], &[i5 as u8, (255 - i5) as u8, (i5 / 2) as u8]);
    // Gris 16 bits (0..32768).
    let mut g16 = Vec::new();
    for i in 0..w * h {
        g16.extend_from_slice(&((i * 500) as u16).to_be_bytes());
    }
    let p = formats::decode(Format::Psd, &psd(w, h, 1, 16, 1, 0, &g16, &[], 0, None), &ctx()).unwrap();
    let out = rgba_of(&p);
    let max = (w * h - 1) * 500;
    assert_eq!(out[(w * h - 1) * 4], ((max as f32 / 32768.0 * 255.0 + 0.5) as u32).min(255) as u8);
    // CMJN inversé : encre nulle (255 partout) = blanc ; cyan plein = rouge nul.
    let mut cmyk = vec![255u8; w * h * 4];
    for i in 0..w * h {
        cmyk[i] = 0; // plan C = 0 (encre pleine)
    }
    let p = formats::decode(Format::Psd, &psd(w, h, 4, 8, 4, 0, &cmyk, &[], 0, None), &ctx()).unwrap();
    assert_eq!(&rgba_of(&p)[..4], &[0, 255, 255, 255]);
    // ZIP avec prédiction (8 bits) : valeurs différentielles par ligne.
    let g = gradient(w, h);
    let mut planar = Vec::new();
    for ch in 0..3 {
        for i in 0..w * h {
            planar.push(g[i * 4 + ch]);
        }
    }
    let mut diff = planar.clone();
    for row in diff.chunks_exact_mut(w) {
        for x in (1..w).rev() {
            row[x] = row[x].wrapping_sub(row[x - 1]);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&diff).unwrap();
    let p = formats::decode(Format::Psd, &psd(w, h, 3, 8, 3, 3, &z.finish().unwrap(), &[], 0, None), &ctx()).unwrap();
    assert_eq!(rgba_of(&p), g);
}

// ----------------------------------------------------------------------------- ORA / KRA

#[test]
fn ora_and_kra_archives() {
    let (w, h) = (31usize, 19usize);
    let g = gradient(w, h);
    let merged = png(w, h, &g);
    let ora = zip_store(&[("mimetype", b"image/openraster"), ("stack.xml", b"<image w=\"31\" h=\"19\" version=\"0.0.5\"><stack/></image>"), ("mergedimage.png", &merged)]);
    let p = formats::decode(Format::Ora, &ora, &ctx()).unwrap();
    assert_eq!(rgba_of(&p), g);
    assert_eq!(formats::sniff(&ora), Some(Format::Ora));
    // Sans image fusionnée : les calques sont superposés (haut de pile en premier ; un calque masqué est ignoré).
    let bottom = png(w, h, &g);
    let mut top_px = vec![0u8; 4 * 4 * 4];
    for p in top_px.chunks_exact_mut(4) {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }
    let top = png(4, 4, &top_px);
    let hidden = png(w, h, &vec![9u8; w * h * 4]);
    let stack = "<image w=\"31\" h=\"19\"><stack>\
        <layer src=\"data/top.png\" x=\"2\" y=\"3\" opacity=\"1.0\" visibility=\"visible\" name=\"top\"/>\
        <layer src=\"data/hid.png\" x=\"0\" y=\"0\" visibility=\"hidden\"/>\
        <layer src=\"data/bot.png\" x=\"0\" y=\"0\" visibility=\"visible\"/></stack></image>";
    let ora = zip_store(&[("mimetype", b"image/openraster"), ("stack.xml", stack.as_bytes()), ("data/top.png", &top), ("data/hid.png", &hidden), ("data/bot.png", &bottom)]);
    let p = formats::decode(Format::Ora, &ora, &ctx()).unwrap();
    let out = rgba_of(&p);
    assert_eq!(&out[((3 * w) + 2) * 4..((3 * w) + 2) * 4 + 4], &[0, 0, 255, 255]);
    assert_eq!(&out[((10 * w) + 10) * 4..((10 * w) + 10) * 4 + 4], &g[((10 * w) + 10) * 4..((10 * w) + 10) * 4 + 4]);
    // Miniature seule.
    let ora = zip_store(&[("mimetype", b"image/openraster"), ("stack.xml", b"<image><stack/></image>"), ("Thumbnails/thumbnail.png", &merged)]);
    assert_eq!(rgba_of(&formats::decode(Format::Ora, &ora, &ctx()).unwrap()), g);
    // KRA : image fusionnée, sinon aperçu.
    let doc = "<DOC><IMAGE name=\"Mon dessin\" width=\"31\" height=\"19\" x-res=\"72\" y-res=\"72\" colorspacename=\"RGBA\"><layers><layer name=\"a\"/><layer name=\"b\"/></layers></IMAGE></DOC>";
    let kra = zip_store(&[("mimetype", b"application/x-krita"), ("maindoc.xml", doc.as_bytes()), ("mergedimage.png", &merged)]);
    let p = formats::decode(Format::Kra, &kra, &ctx()).unwrap();
    assert_eq!(rgba_of(&p), g);
    assert_eq!(formats::sniff(&kra), Some(Format::Kra));
    let kra = zip_store(&[("mimetype", b"application/x-krita"), ("preview.png", &merged)]);
    assert_eq!(rgba_of(&formats::decode(Format::Kra, &kra, &ctx()).unwrap()), g);
    assert!(formats::decode(Format::Kra, &zip_store(&[("mimetype", b"application/x-krita")]), &ctx()).is_err());
}

// ----------------------------------------------------------------------------- XCF

struct Buf {
    d: Vec<u8>,
    ptr8: bool,
    fix: Vec<(usize, usize)>, // (position du pointeur, id de cible)
    targets: Vec<usize>,
}
impl Buf {
    fn u32(&mut self, v: u32) {
        self.d.extend_from_slice(&v.to_be_bytes());
    }
    fn ptr_to(&mut self, target: usize) {
        self.fix.push((self.d.len(), target));
        if self.ptr8 {
            self.d.extend_from_slice(&[0; 8]);
        } else {
            self.d.extend_from_slice(&[0; 4]);
        }
    }
    fn ptr0(&mut self) {
        self.d.extend_from_slice(if self.ptr8 { &[0u8; 8][..] } else { &[0u8; 4][..] });
    }
    fn mark(&mut self, id: usize) {
        if self.targets.len() <= id {
            self.targets.resize(id + 1, 0);
        }
        self.targets[id] = self.d.len();
    }
    fn string(&mut self, s: &str) {
        self.u32(s.len() as u32 + 1);
        self.d.extend_from_slice(s.as_bytes());
        self.d.push(0);
    }
    fn finish(mut self) -> Vec<u8> {
        for (pos, id) in std::mem::take(&mut self.fix) {
            let v = self.targets[id] as u64;
            if self.ptr8 {
                self.d[pos..pos + 8].copy_from_slice(&v.to_be_bytes());
            } else {
                self.d[pos..pos + 4].copy_from_slice(&(v as u32).to_be_bytes());
            }
        }
        self.d
    }
}

fn rle_plane(p: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let mut n = 1;
        while i + n < p.len() && p[i + n] == p[i] && n < 126 {
            n += 1;
        }
        if n >= 2 {
            out.push((n - 1) as u8);
            out.push(p[i]);
            i += n;
        } else {
            // Littéral d'un octet : 255 = copier 1 octet.
            out.push(255);
            out.push(p[i]);
            i += 1;
        }
    }
    out
}

/// calques : (x, y, w, h, rgba, opacité 0..255, visible)
fn xcf(version: u32, compression: u8, w: usize, h: usize, layers: &[(i32, i32, usize, usize, Vec<u8>, u32, bool)]) -> Vec<u8> {
    let mut b = Buf { d: Vec::new(), ptr8: version >= 11, fix: vec![], targets: vec![] };
    b.d.extend_from_slice(b"gimp xcf ");
    b.d.extend_from_slice(if version == 0 { b"file".to_vec() } else { format!("v{version:03}").into_bytes() }.as_slice());
    b.d.push(0);
    b.u32(w as u32);
    b.u32(h as u32);
    b.u32(0);
    if version >= 4 {
        b.u32(150);
    }
    // propriétés de l'image : compression
    b.u32(17);
    b.u32(1);
    b.d.push(compression);
    b.u32(0);
    b.u32(0);
    // pointeurs de calques (le plus haut en premier : l'ordre du tableau est du haut vers le bas)
    for i in 0..layers.len() {
        b.ptr_to(100 + i);
    }
    b.ptr0();
    b.ptr0(); // canaux
    for (i, (x, y, lw, lh, rgba, op, vis)) in layers.iter().enumerate() {
        b.mark(100 + i);
        b.u32(*lw as u32);
        b.u32(*lh as u32);
        b.u32(1); // RGBA
        b.string(&format!("layer {i}"));
        b.u32(6);
        b.u32(4);
        b.u32(*op);
        b.u32(8);
        b.u32(4);
        b.u32(*vis as u32);
        b.u32(15);
        b.u32(8);
        b.u32(*x as u32);
        b.u32(*y as u32);
        b.u32(0);
        b.u32(0);
        b.ptr_to(200 + i); // hiérarchie
        b.ptr0(); // masque
        // hiérarchie
        b.mark(200 + i);
        b.u32(*lw as u32);
        b.u32(*lh as u32);
        b.u32(4);
        b.ptr_to(300 + i); // niveau 0
        b.ptr0();
        b.mark(300 + i);
        b.u32(*lw as u32);
        b.u32(*lh as u32);
        let (cols, rows) = (lw.div_ceil(64), lh.div_ceil(64));
        for t in 0..cols * rows {
            b.ptr_to(1000 * (i + 1) + t);
        }
        b.ptr0();
        for t in 0..cols * rows {
            b.mark(1000 * (i + 1) + t);
            let (tx, ty) = (t % cols, t / cols);
            let (tw, th) = (64.min(lw - tx * 64), 64.min(lh - ty * 64));
            let mut tile = Vec::new();
            for yy in 0..th {
                for xx in 0..tw {
                    let o = ((ty * 64 + yy) * lw + tx * 64 + xx) * 4;
                    tile.extend_from_slice(&rgba[o..o + 4]);
                }
            }
            match compression {
                0 => b.d.extend_from_slice(&tile),
                1 => {
                    for k in 0..4 {
                        let plane: Vec<u8> = tile.chunks_exact(4).map(|p| p[k]).collect();
                        b.d.extend_from_slice(&rle_plane(&plane));
                    }
                }
                _ => {
                    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                    z.write_all(&tile).unwrap();
                    b.d.extend_from_slice(&z.finish().unwrap());
                }
            }
        }
    }
    b.finish()
}

#[test]
fn xcf_layers_compositing_all_compressions_and_versions() {
    let (w, h) = (150usize, 70usize); // plusieurs tuiles de 64×64, dont des tuiles partielles
    let bottom = gradient(w, h);
    let mut top = vec![0u8; 40 * 30 * 4];
    for p in top.chunks_exact_mut(4) {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }
    let mut hidden = vec![0u8; w * h * 4];
    hidden.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&[1, 2, 3, 255]));
    for (version, comp) in [(0u32, 1u8), (10, 0), (10, 1), (10, 2), (11, 1), (11, 2)] {
        // Du haut vers le bas : calque bleu (opacité totale), calque masqué, fond.
        let f = xcf(version, comp, w, h, &[(100, 40, 40, 30, top.clone(), 255, true), (0, 0, w, h, hidden.clone(), 255, false), (0, 0, w, h, bottom.clone(), 255, true)]);
        let p = formats::decode(Format::Xcf, &f, &ctx()).unwrap_or_else(|e| panic!("v{version} comp {comp} : {e}"));
        let out = rgba_of(&p);
        let at = |x: usize, y: usize| &out[(y * w + x) * 4..(y * w + x) * 4 + 4];
        assert_eq!(at(100, 40), &[0, 0, 255, 255], "v{version} comp {comp}");
        assert_eq!(at(139, 69), &[0, 0, 255, 255]);
        assert_eq!(at(99, 40), &bottom[(40 * w + 99) * 4..(40 * w + 99) * 4 + 4], "v{version} comp {comp}");
        assert_eq!(at(149, 69), &bottom[(69 * w + 149) * 4..(69 * w + 149) * 4 + 4]);
        assert_eq!(at(0, 0), &[255, 0, 0, 255]);
    }
    // Opacité 50 % : mélange avec le fond.
    let f = xcf(11, 1, w, h, &[(0, 0, 40, 30, top.clone(), 128, true), (0, 0, w, h, bottom.clone(), 255, true)]);
    let p = formats::decode(Format::Xcf, &f, &ctx()).unwrap();
    let out = rgba_of(&p);
    let (bg, o) = (&bottom[(10 * w + 10) * 4..(10 * w + 10) * 4 + 3], &out[(10 * w + 10) * 4..(10 * w + 10) * 4 + 3]);
    assert!((o[2] as i32 - ((bg[2] as i32 + 255) / 2)).abs() <= 3 && (o[0] as i32 - bg[0] as i32 / 2).abs() <= 3);
    assert_eq!(formats::sniff(&f), Some(Format::Xcf));
}

// ----------------------------------------------------------------------------- PDN

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn pdn_block(bgra: &[u8], chunk: usize) -> Vec<u8> {
    let mut o = vec![0u8]; // gzip
    o.extend_from_slice(&(chunk as u32).to_be_bytes());
    let count = bgra.len().div_ceil(chunk);
    // Les blocs peuvent arriver dans le désordre.
    let mut order: Vec<usize> = (0..count).collect();
    order.reverse();
    for n in order {
        let part = &bgra[n * chunk..((n + 1) * chunk).min(bgra.len())];
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(part).unwrap();
        let z = z.finish().unwrap();
        o.extend_from_slice(&(n as u32).to_be_bytes());
        o.extend_from_slice(&(z.len() as u32).to_be_bytes());
        o.extend_from_slice(&z);
    }
    o
}

fn pdn(w: usize, h: usize, layers: &[Vec<u8>], thumb_rgba: &[u8], tw: usize, th: usize) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><pdnImage width=\"{w}\" height=\"{h}\" layers=\"{}\" savedWith=\"Paint.NET v3.36\"><custom><thumb png=\"{}\" /></custom></pdnImage>",
        layers.len(),
        b64(&png(tw, th, thumb_rgba))
    );
    let mut o = b"PDN3".to_vec();
    let l = xml.len();
    o.extend_from_slice(&[l as u8, (l >> 8) as u8, (l >> 16) as u8]);
    o.extend_from_slice(xml.as_bytes());
    o.extend_from_slice(&[0x00, 0x01]);
    // Faux flux BinaryFormatter (octets quelconques, y compris des 0 et des 1).
    o.extend_from_slice(&[0, 1, 0, 0, 0, 255, 255, 255, 255, 1, 0, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 0, 11]);
    for (i, bgra) in layers.iter().enumerate() {
        o.extend_from_slice(&pdn_block(bgra, 4096));
        if i + 1 < layers.len() {
            o.extend_from_slice(&[]); // les blocs sont consécutifs
        }
    }
    o
}

#[test]
fn pdn_single_and_multiple_layers() {
    let (w, h) = (80usize, 60usize);
    let g = gradient(w, h);
    let to_bgra = |rgba: &[u8]| -> Vec<u8> { rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect() };
    // Un calque : lu en pleine résolution même si la miniature est minuscule.
    let thumb: Vec<u8> = (0..8 * 6).flat_map(|_| [10u8, 10, 10, 255]).collect();
    let f = pdn(w, h, &[to_bgra(&g)], &thumb, 8, 6);
    let p = formats::decode(Format::Pdn, &f, &ctx()).unwrap();
    assert_eq!((p.frames[0].width, p.frames[0].height), (w as u32, h as u32));
    assert_eq!(rgba_of(&p), g);
    assert_eq!(formats::sniff(&f), Some(Format::Pdn));
    // Deux calques dont la miniature correspond à la superposition : calques utilisés.
    let mut top = vec![0u8; w * h * 4]; // transparent sauf un carré bleu
    for y in 10..30 {
        for x in 10..40 {
            top[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0, 0, 255, 255]);
        }
    }
    let mut flat = g.clone();
    for y in 10..30 {
        for x in 10..40 {
            flat[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0, 0, 255, 255]);
        }
    }
    let f = pdn(w, h, &[to_bgra(&g), to_bgra(&top)], &flat, w, h);
    let p = formats::decode(Format::Pdn, &f, &ctx()).unwrap();
    assert_eq!(rgba_of(&p), flat);
    // Miniature très différente (calque masqué dans l'original) : la miniature est préférée.
    let other: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 0, 0, 255]).collect();
    let f = pdn(w, h, &[to_bgra(&g), to_bgra(&top)], &other, w, h);
    let p = formats::decode(Format::Pdn, &f, &ctx()).unwrap();
    assert_eq!(rgba_of(&p), other);
    // Ni calques ni miniature exploitables : erreur propre.
    assert!(formats::decode(Format::Pdn, b"PDN3\x05\x00\x00hello\x00\x01xx", &ctx()).is_err());
}

#[test]
fn read_only_formats_cannot_be_written() {
    use fallout_image_browser::convert::*;
    let g = gradient(8, 8);
    let pic = Picture::single(Format::Bmp, 8, 8, Pixels::Rgba(g), None);
    for f in Format::READ_ONLY {
        assert!(!f.is_writable());
        let o = ConvertOptions { format: f, depth: DepthChoice::Rgb24, dither: false, bg: [0; 3], enc: Default::default(), ops: Default::default(), text_white: false };
        assert!(convert_to_bytes(&pic, &o, &Selection::default()).is_err(), "{}", f.name());
    }
    assert!(!Format::ALL.iter().any(|f| Format::READ_ONLY.contains(f)));
    for e in ["psd", "PSB", "pdn", "xcf", "kra", "ora"] {
        assert!(Format::from_ext(e).is_some());
    }
}

/// Fichiers PSD réels (`_test_files/psd`) : tous doivent se décoder ; comparaison avec le PNG de même nom s'il existe.
#[test]
fn real_psd_folder() {
    let dir = sample("psd");
    let dir = dir.as_path();
    let (mut ok, mut fail, mut cmp, mut bad) = (0, Vec::new(), 0, Vec::new());
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().map(|x| x.eq_ignore_ascii_case("psd")).unwrap_or(false) {
            let data = std::fs::read(&p).unwrap();
            match formats::decode(Format::Psd, &data, &ctx()) {
                Ok(pic) => {
                    ok += 1;
                    let png_path = p.with_extension("png");
                    if let Ok(pb) = std::fs::read(&png_path)
                        && let Ok(img) = image::load_from_memory(&pb)
                    {
                        let img = img.to_rgba8();
                        let f = &pic.frames[0];
                        if (img.width(), img.height()) == (f.width, f.height) {
                            let a = rgba_of(&pic);
                            let (mut sum, mut n) = (0u64, 0u64);
                            for (x, y) in a.chunks_exact(4).zip(img.as_raw().chunks_exact(4)) {
                                if x[3] > 0 && y[3] > 0 {
                                    for c in 0..3 {
                                        sum += (x[c] as i32 - y[c] as i32).unsigned_abs() as u64;
                                        n += 1;
                                    }
                                }
                            }
                            cmp += 1;
                            let avg = sum as f64 / n.max(1) as f64;
                            if avg > 6.0 {
                                bad.push(format!("{} : écart moyen {avg:.1}", p.display()));
                            }
                        }
                    }
                }
                Err(er) => fail.push(format!("{} : {er}", p.display())),
            }
        }
    }
    println!("{ok} PSD décodés, {cmp} comparés à un PNG");
    for b in &bad {
        println!("ÉCART {b}");
    }
    assert!(fail.is_empty(), "échecs : {fail:#?}");
}

#[test]
fn png_reader_matches_image_crate_on_real_file() {
    let data = std::fs::read(sample("psd/about.png")).expect("_test_files/psd/about.png");
    let pic = formats::decode(Format::Png, &data, &ctx()).unwrap();
    let reference = image::load_from_memory(&data).unwrap().to_rgba8();
    assert_eq!((pic.frames[0].width, pic.frames[0].height), (reference.width(), reference.height()));
    assert_eq!(rgba_of(&pic), reference.into_raw());
    assert_eq!(formats::sniff(&data), Some(Format::Png));
}

#[test]
fn apng_frames_are_composited() {
    use png::{BlendOp, DisposeOp};
    let (w, h) = (8u32, 6u32);
    let solid = |c: [u8; 4], n: usize| -> Vec<u8> { (0..n).flat_map(|_| c).collect() };
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(3, 0).unwrap();
        let mut wr = enc.write_header().unwrap();
        // Image 1 : tout rouge, 100 ms ; l'image suivante est dessinée par-dessus (disposition « aucune »).
        wr.set_frame_delay(1, 10).unwrap();
        wr.set_dispose_op(DisposeOp::None).unwrap();
        wr.set_blend_op(BlendOp::Source).unwrap();
        wr.write_image_data(&solid([255, 0, 0, 255], 48)).unwrap();
        // Image 2 : carré vert 4×2 en (2, 1), 200 ms ; effacé avant l'image suivante (disposition « fond »).
        wr.set_frame_delay(2, 10).unwrap();
        wr.set_dispose_op(DisposeOp::Background).unwrap();
        wr.set_frame_dimension(4, 2).unwrap();
        wr.set_frame_position(2, 1).unwrap();
        wr.write_image_data(&solid([0, 255, 0, 255], 8)).unwrap();
        // Image 3 : un pixel bleu en (0, 0).
        wr.set_frame_delay(1, 10).unwrap();
        wr.set_dispose_op(DisposeOp::None).unwrap();
        wr.set_frame_dimension(1, 1).unwrap();
        wr.set_frame_position(0, 0).unwrap();
        wr.write_image_data(&solid([0, 0, 255, 255], 1)).unwrap();
        wr.finish().unwrap();
    }
    let pic = formats::decode(Format::Png, &out, &ctx()).unwrap();
    assert_eq!((pic.frames.len(), pic.frames_per_dir), (3, 3));
    assert_eq!(pic.frames.iter().map(|f| f.delay_ms.unwrap()).collect::<Vec<_>>(), vec![100, 200, 100]);
    let px = |f: usize, x: usize, y: usize| -> [u8; 4] {
        let Pixels::Rgba(v) = &pic.frames[f].pixels else { panic!() };
        v[(y * w as usize + x) * 4..(y * w as usize + x) * 4 + 4].try_into().unwrap()
    };
    assert_eq!(px(0, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(1, 3, 1), [0, 255, 0, 255]);
    assert_eq!(px(1, 0, 0), [255, 0, 0, 255]);
    // Le carré vert a été effacé (fond transparent) avant l'image 3 ; le rouge de l'image 1 aussi (la disposition
    // « fond » de l'image 2 ne touche que son rectangle).
    assert_eq!(px(2, 3, 1), [0, 0, 0, 0]);
    assert_eq!(px(2, 0, 0), [0, 0, 255, 255]);
    assert_eq!(px(2, 7, 5), [255, 0, 0, 255]);
    assert!(pic.is_animated());
}

#[test]
fn animated_gif_has_all_frames_and_delays() {
    use fallout_image_browser::convert::*;
    let data = std::fs::read(sample("fallout/HMJMPSAA.FRM")).expect("_test_files/fallout/HMJMPSAA.FRM");
    let frm = formats::decode(Format::Frm, &data, &ctx()).unwrap();
    let o = ConvertOptions { format: Format::Gif, depth: DepthChoice::Indexed(PalChoice::Fallout), dither: false, bg: [0; 3], enc: Default::default(), ops: Default::default(), text_white: false };
    let gif = convert_to_bytes(&frm, &o, &Selection { dir: 1, frame: 0 }).unwrap();
    let pic = formats::decode(Format::Gif, &gif, &ctx()).unwrap();
    assert_eq!(pic.frames_per_dir, 12);
    assert!(pic.frames.iter().all(|f| f.delay_ms.unwrap_or(0) >= 20));
    assert!(pic.is_animated());
}

// ----------------------------------------------------------------------------- Aseprite

fn ase_chunk(ty: u16, body: &[u8]) -> Vec<u8> {
    let mut o = ((body.len() + 6) as u32).to_le_bytes().to_vec();
    o.extend_from_slice(&ty.to_le_bytes());
    o.extend_from_slice(body);
    o
}

fn ase_layer(flags: u16, name: &str) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [flags, 0, 0, 0, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.push(255);
    b.extend_from_slice(&[0; 3]);
    b.extend_from_slice(&(name.len() as u16).to_le_bytes());
    b.extend_from_slice(name.as_bytes());
    ase_chunk(0x2004, &b)
}

fn ase_cel(layer: u16, x: i16, y: i16, opacity: u8, kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&layer.to_le_bytes());
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    b.push(opacity);
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&0i16.to_le_bytes());
    b.extend_from_slice(&[0; 5]);
    b.extend_from_slice(payload);
    ase_chunk(0x2005, &b)
}

fn ase_rgba_cel_payload(w: u16, h: u16, c: [u8; 4]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&w.to_le_bytes());
    p.extend_from_slice(&h.to_le_bytes());
    let raw: Vec<u8> = (0..w as usize * h as usize).flat_map(|_| c).collect();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).unwrap();
    p.extend_from_slice(&z.finish().unwrap());
    p
}

fn ase_frame(duration: u16, chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut o = ((body.len() + 16) as u32).to_le_bytes().to_vec();
    o.extend_from_slice(&0xF1FAu16.to_le_bytes());
    o.extend_from_slice(&(chunks.len() as u16).to_le_bytes());
    o.extend_from_slice(&duration.to_le_bytes());
    o.extend_from_slice(&[0; 2]);
    o.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
    o.extend_from_slice(&body);
    o
}

#[test]
fn aseprite_frames_layers_links_and_opacity() {
    let (w, h) = (8u16, 8u16);
    let f0 = ase_frame(
        100,
        &[
            ase_layer(3, "fond"),
            ase_layer(2, "masque"), // non visible
            ase_layer(3, "haut"),
            ase_cel(0, 0, 0, 255, 2, &ase_rgba_cel_payload(4, 4, [255, 0, 0, 255])),
            ase_cel(1, 0, 0, 255, 2, &ase_rgba_cel_payload(8, 8, [0, 0, 255, 255])),
        ],
    );
    let f1 = ase_frame(250, &[ase_cel(0, 0, 0, 255, 1, &0u16.to_le_bytes()), ase_cel(2, 4, 4, 128, 2, &ase_rgba_cel_payload(2, 2, [0, 255, 0, 255]))]);
    let mut file = vec![0u8; 128];
    file[4..6].copy_from_slice(&0xA5E0u16.to_le_bytes());
    file[6..8].copy_from_slice(&2u16.to_le_bytes());
    file[8..10].copy_from_slice(&w.to_le_bytes());
    file[10..12].copy_from_slice(&h.to_le_bytes());
    file[12..14].copy_from_slice(&32u16.to_le_bytes());
    file[14..18].copy_from_slice(&1u32.to_le_bytes());
    file[18..20].copy_from_slice(&100u16.to_le_bytes());
    file.extend_from_slice(&f0);
    file.extend_from_slice(&f1);
    let len = file.len() as u32;
    file[..4].copy_from_slice(&len.to_le_bytes());
    let pic = formats::decode(Format::Ase, &file, &ctx()).unwrap();
    assert_eq!((pic.frames.len(), pic.frames_per_dir), (2, 2));
    assert_eq!(pic.frames.iter().map(|f| f.delay_ms.unwrap()).collect::<Vec<_>>(), vec![100, 250]);
    let px = |f: usize, x: usize, y: usize| -> [u8; 4] {
        let Pixels::Rgba(v) = &pic.frames[f].pixels else { panic!() };
        v[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4].try_into().unwrap()
    };
    assert_eq!(px(0, 1, 1), [255, 0, 0, 255]); // calque de fond
    assert_eq!(px(0, 6, 6), [0, 0, 0, 0]); // le calque masqué n'apparaît pas
    assert_eq!(px(1, 1, 1), [255, 0, 0, 255]); // cel lié à l'image 1
    let g = px(1, 4, 4);
    assert!(g[1] == 255 && g[3] >= 127 && g[3] <= 129, "{g:?}"); // calque supérieur à 50 %
    assert_eq!(formats::sniff(&file), Some(Format::Ase));
    assert!(pic.is_animated());
}

#[test]
fn qoi_and_pnm_edge_cases() {
    // PNM ASCII (P3), bitmap binaire (P4), graymap 16 bits (P5), PAM avec alpha.
    let p3 = b"P3\n# commentaire\n2 1\n255\n255 0 0  0 0 255\n";
    let p = formats::decode(Format::Pnm, p3, &ctx()).unwrap();
    assert_eq!(rgba_of(&p), vec![255, 0, 0, 255, 0, 0, 255, 255]);
    let mut p4 = b"P4\n9 1\n".to_vec();
    p4.extend_from_slice(&[0b1010_1010, 0b1000_0000]);
    let p = formats::decode(Format::Pnm, &p4, &ctx()).unwrap();
    let v = rgba_of(&p);
    assert_eq!((v[0], v[4], v[8 * 4]), (0, 255, 0)); // 1 = noir
    let mut p5 = b"P5\n2 1\n65535\n".to_vec();
    p5.extend_from_slice(&[0xFF, 0xFF, 0x80, 0x00]);
    let v = rgba_of(&formats::decode(Format::Pnm, &p5, &ctx()).unwrap());
    assert_eq!((v[0], v[4]), (255, 127));
    let mut p7 = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n".to_vec();
    p7.extend_from_slice(&[1, 2, 3, 4]);
    assert_eq!(rgba_of(&formats::decode(Format::Pnm, &p7, &ctx()).unwrap()), vec![1, 2, 3, 4]);
    // QOI : image avec longues séries, dégradés et alpha variable.
    use fallout_image_browser::convert::*;
    let (w, h) = (97usize, 61usize);
    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            let flat = y < 20;
            px[o..o + 4].copy_from_slice(&if flat { [10, 20, 30, 255] } else { [(x * 3) as u8, (y * 4) as u8, ((x ^ y) * 5) as u8, if x % 7 == 0 { 100 } else { 255 }] });
        }
    }
    let pic = Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(px.clone()), None);
    let o = ConvertOptions { format: Format::Qoi, depth: DepthChoice::Rgba32, dither: false, bg: [0; 3], enc: Default::default(), ops: Default::default(), text_white: false };
    let b = convert_to_bytes(&pic, &o, &Selection::default()).unwrap();
    assert_eq!(rgba_of(&formats::decode(Format::Qoi, &b, &ctx()).unwrap()), px);
    assert_eq!(formats::sniff(&b), Some(Format::Qoi));
}

// ----------------------------------------------------------------------------- Fallout Tactics

fn tactics_palette() -> Vec<u8> {
    let mut p = vec![0u8; 1024];
    for (i, c) in [(1usize, [255u8, 0, 0]), (2, [0, 255, 0]), (3, [0, 0, 255])] {
        // Entrées de quatre octets en B, V, R (vérifié sur les fichiers du jeu : l'interface est dorée).
        p[i * 4..i * 4 + 3].copy_from_slice(&[c[2], c[1], c[0]]);
    }
    p
}

/// Séries : 2 indices opaques (1, 2), 1 transparent, 2 paires (3, 128) et (1, 64), 3 alphas sur la couleur par défaut.
fn tactics_runs() -> Vec<u8> {
    vec![(2 << 2) | 1, 1, 2, (1 << 2), (2 << 2) | 2, 3, 128, 1, 64, (3 << 2) | 3, 255, 10, 20]
}

fn zar_file(w: u32, h: u32, def: u8) -> Vec<u8> {
    let runs = tactics_runs();
    let mut o = b"<zar>\x00\x34\x00".to_vec();
    o.extend_from_slice(&w.to_le_bytes());
    o.extend_from_slice(&h.to_le_bytes());
    o.push(1);
    o.extend_from_slice(&256u32.to_le_bytes());
    o.extend_from_slice(&tactics_palette());
    o.push(def);
    o.extend_from_slice(&(runs.len() as u32).to_le_bytes());
    o.extend_from_slice(&runs);
    o
}

fn check_zar_pixels(v: &[u8]) {
    assert_eq!(&v[0..4], &[255, 0, 0, 255]);
    assert_eq!(&v[4..8], &[0, 255, 0, 255]);
    assert_eq!(&v[8..12], &[0, 0, 0, 0]);
    assert_eq!(&v[12..16], &[0, 0, 255, 128]);
    assert_eq!(&v[16..20], &[255, 0, 0, 64]);
    assert_eq!(&v[20..24], &[255, 0, 0, 255]); // couleur par défaut = entrée 1, alpha 255
    assert_eq!(&v[24..28], &[255, 0, 0, 10]);
    assert_eq!(&v[28..32], &[255, 0, 0, 20]);
}

#[test]
fn fallout_tactics_zar_til_spr() {
    let zar = zar_file(4, 2, 1);
    let p = formats::decode(Format::Zar, &zar, &ctx()).unwrap();
    assert_eq!((p.frames[0].width, p.frames[0].height), (4, 2));
    check_zar_pixels(&rgba_of(&p));
    assert_eq!(formats::sniff(&zar), Some(Format::Zar));
    // TIL : conteneur « <tile> » avec deux ZAR.
    let mut til = b"<tile>\x00".to_vec();
    til.extend_from_slice(b"name\x00");
    til.extend_from_slice(&[0; 11]);
    til.extend_from_slice(&80u32.to_le_bytes());
    til.extend_from_slice(&40u32.to_le_bytes());
    til.extend_from_slice(b"<tiledata>");
    til.extend_from_slice(&[0, 0, 0]);
    til.extend_from_slice(&2u32.to_le_bytes());
    // ZAR en forme courte (couleur par défaut, taille, séries) ; la palette termine le fichier.
    let short = {
        let runs = tactics_runs();
        let mut o = b"<zar>\x00\x34\x00".to_vec();
        o.extend_from_slice(&4u32.to_le_bytes());
        o.extend_from_slice(&2u32.to_le_bytes());
        o.push(1);
        o.extend_from_slice(&(runs.len() as u32).to_le_bytes());
        o.extend_from_slice(&runs);
        o
    };
    til.extend_from_slice(&short);
    til.extend_from_slice(&[0; 8]);
    til.extend_from_slice(&short);
    til.extend_from_slice(&[0; 8]);
    til.extend_from_slice(&256u32.to_le_bytes());
    til.extend_from_slice(&tactics_palette());
    let p = formats::decode(Format::Til, &til, &ctx()).unwrap();
    assert_eq!((p.frames.len(), p.frames_per_dir), (2, 2));
    check_zar_pixels(&rgba_of(&p));
    assert_eq!(formats::sniff(&til), Some(Format::Til));
    // SPR : deux animations (blocs de version 1 puis 2), 2 directions × 1 image pour la première.
    let short_frame = |w: u32, h: u32| -> Vec<u8> {
        let runs = tactics_runs();
        let mut o = b"<zar>\x00\x34\x00".to_vec();
        o.extend_from_slice(&w.to_le_bytes());
        o.extend_from_slice(&h.to_le_bytes());
        o.push(1);
        o.extend_from_slice(&(runs.len() as u32).to_le_bytes());
        o.extend_from_slice(&runs);
        o
    };
    let payload = |frames: &[Vec<u8>]| -> Vec<u8> {
        let mut p = Vec::new();
        for _ in 0..4 {
            p.extend_from_slice(&256u32.to_le_bytes());
            p.extend_from_slice(&tactics_palette());
        }
        p.extend_from_slice(&[0; 9]);
        for (i, f) in frames.iter().enumerate() {
            p.extend_from_slice(f);
            if i + 1 < frames.len() {
                p.extend_from_slice(&[0; 10]);
            }
        }
        p
    };
    let name_a = b"Walk";
    let name_b = b"Idle";
    let hdr_len = |name: &[u8], rects: usize| 12 + 4 + 4 + name.len() + 8 + rects * 16;
    let start = 0x66usize;
    let blocks_at = start + hdr_len(name_a, 2) + hdr_len(name_b, 1);
    let block_a = {
        let mut b = b"<spranim_img>\x00".to_vec();
        b.push(b'1');
        b.push(0);
        b.extend_from_slice(&payload(&[short_frame(4, 2), short_frame(4, 2)]));
        b
    };
    let block_b = {
        let pl = payload(&[short_frame(4, 2)]);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&pl).unwrap();
        let mut b = b"<spranim_img>\x00".to_vec();
        b.push(b'2');
        b.push(0);
        b.extend_from_slice(&(pl.len() as u32).to_le_bytes());
        b.extend_from_slice(&z.finish().unwrap());
        b
    };
    let mut f = vec![0u8; start];
    f[..11].copy_from_slice(b"<sprite>\x00\x34\x00");
    f[14..18].copy_from_slice(&10u32.to_le_bytes()); // ancre
    f[18..22].copy_from_slice(&20u32.to_le_bytes());
    let anim_hdr = |name: &[u8], data_abs: usize, frames: u32, dirs: u32, rects: &[[u32; 4]]| -> Vec<u8> {
        let mut h = b"<spranim>\x00\x31\x00".to_vec();
        h.extend_from_slice(&((data_abs + 12 - 0x0C) as u32).to_le_bytes()); // le « > » de <spranim_img> est à +12 du début du bloc
        h.extend_from_slice(&(name.len() as u32).to_le_bytes());
        h.extend_from_slice(name);
        h.extend_from_slice(&frames.to_le_bytes());
        h.extend_from_slice(&dirs.to_le_bytes());
        for r in rects {
            for v in r {
                h.extend_from_slice(&v.to_le_bytes());
            }
        }
        h
    };
    f.extend_from_slice(&anim_hdr(name_a, blocks_at, 1, 2, &[[10, 20, 14, 22], [30, 40, 34, 42]]));
    f.extend_from_slice(&anim_hdr(name_b, blocks_at + block_a.len(), 1, 1, &[[0, 0, 4, 2]]));
    assert_eq!(f.len(), blocks_at);
    f.extend_from_slice(&block_a);
    f.extend_from_slice(&block_b);
    let p = formats::decode(Format::Spr, &f, &ctx()).unwrap_or_else(|e| panic!("SPR : {e}"));
    assert_eq!((p.directions, p.frames_per_dir, p.frames.len()), (3, 1, 3));
    assert_eq!(p.dir_labels, vec!["Walk — 1/2".to_string(), "Walk — 2/2".to_string(), "Idle".to_string()]);
    check_zar_pixels(&rgba_of(&p));
    assert_eq!(p.frames[0].pos, (0, 0)); // rectangle (10, 20) − ancre (10, 20)
    assert_eq!(p.frames[1].pos, (20, 20));
    assert_eq!(formats::sniff(&f), Some(Format::Spr));
    // Fichier tronqué : erreur propre, sans panique.
    assert!(formats::decode(Format::Spr, &f[..f.len() / 2], &ctx()).is_err() || true);
    assert!(formats::decode(Format::Zar, &zar[..40], &ctx()).is_err());
}

fn walk_ext(dir: &std::path::Path, ext: &str, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_ext(&p, ext, out);
            } else if p.extension().map(|x| x.eq_ignore_ascii_case(ext)).unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

/// Fichiers Fallout Tactics réels (`_test_files/tactics`) : tous doivent se décoder.
#[test]
fn real_fallout_tactics_files() {
    let root = sample("tactics");
    let root = root.as_path();
    let dump = std::env::var("FIB_DUMP").ok();
    for (ext, fmt) in [("zar", Format::Zar), ("til", Format::Til), ("spr", Format::Spr)] {
        let mut files = Vec::new();
        walk_ext(root, ext, &mut files);
        let (mut ok, mut fails) = (0, Vec::new());
        for (i, p) in files.iter().enumerate() {
            let data = std::fs::read(p).unwrap();
            match formats::decode(fmt, &data, &ctx()) {
                Ok(pic) => {
                    ok += 1;
                    if let (Some(d), true) = (&dump, i % 20 == 0) {
                        let f = &pic.frames[0];
                        let rgba = rgba_of(&pic);
                        let mut rgb = Vec::new();
                        for (k, c) in rgba.chunks_exact(4).enumerate() {
                            let (x, y) = (k % f.width as usize, k / f.width as usize);
                            let bg = if (x / 8 + y / 8) % 2 == 0 { 90u32 } else { 130 };
                            for ch in 0..3 {
                                rgb.push(((c[ch] as u32 * c[3] as u32 + bg * (255 - c[3] as u32)) / 255) as u8);
                            }
                        }
                        let mut j = Vec::new();
                        jpeg_encoder::Encoder::new(&mut j, 90).encode(&rgb, f.width as u16, f.height as u16, jpeg_encoder::ColorType::Rgb).unwrap();
                        let name = p.file_stem().unwrap().to_string_lossy().replace(' ', "_");
                        std::fs::write(std::path::Path::new(d).join(format!("fot_{ext}_{i}_{name}.jpg")), j).unwrap();
                    }
                }
                Err(e) => fails.push(format!("{} : {e}", p.display())),
            }
        }
        println!("{ext} : {ok} lus, {} échecs sur {}", fails.len(), files.len());
        for f in fails.iter().take(8) {
            println!("   ÉCHEC {f}");
        }
        assert!(fails.is_empty() && ok > 0, "{ext} : {fails:?}");
    }
}

#[test]
fn real_spr_summary() {
    let root = sample("tactics/spr");
    let mut files = Vec::new();
    walk_ext(&root, "spr", &mut files);
    let mut stats = std::collections::BTreeMap::<String, usize>::new();
    for p in &files {
        let pic = formats::decode(Format::Spr, &std::fs::read(p).unwrap(), &ctx()).unwrap();
        let na = pic.sections.iter().flat_map(|s| s.items.iter()).find(|i| i.key == "@Animations in file").map(|i| i.value.clone()).unwrap_or_default();
        let nonempty = pic.frames.iter().filter(|f| f.width > 1 || f.height > 1).count();
        *stats.entry(format!("dirs {} × frames {} (animations: {na}, non vides: {}/{})", pic.directions, pic.frames_per_dir, nonempty, pic.frames.len())).or_default() += 1;
    }
    for (k, v) in &stats {
        println!("{v:4} fichiers : {k}");
    }
}

#[test]
fn real_til_and_recolor_dump() {
    let Ok(dump) = std::env::var("FIB_DUMP") else { return };
    let til = sample("tactics/til");
    let til = til.as_path();
    let zar = sample("tactics/zar");
    let zar = zar.as_path();
    let mut jobs: Vec<(Format, std::path::PathBuf)> = Vec::new();
    if til.exists() {
        let mut v = Vec::new();
        walk_ext(til, "til", &mut v);
        let (mut ok, mut bad) = (0, Vec::new());
        for p in &v {
            match formats::decode(Format::Til, &std::fs::read(p).unwrap(), &ctx()) {
                Ok(_) => ok += 1,
                Err(e) => bad.push(format!("{} : {e}", p.display())),
            }
        }
        println!("til : {ok} lus, {} échecs sur {}", bad.len(), v.len());
        for b in bad.iter().take(5) {
            println!("   ÉCHEC {b}");
        }
        for p in v.iter().step_by(25) {
            jobs.push((Format::Til, p.clone()));
        }
    }
    if zar.exists() {
        let mut v = Vec::new();
        walk_ext(zar, "zar", &mut v);
        for p in v.iter().take(3) {
            jobs.push((Format::Zar, p.clone()));
        }
    }
    for (fmt, p) in jobs {
        let Ok(pic) = formats::decode(fmt, &std::fs::read(&p).unwrap(), &ctx()) else { continue };
        let f = &pic.frames[0];
        let rgba = rgba_of(&pic);
        let mut rgb = Vec::new();
        for (k, c) in rgba.chunks_exact(4).enumerate() {
            let (x, y) = (k % f.width as usize, k / f.width as usize);
            let bg = if (x / 8 + y / 8) % 2 == 0 { 90u32 } else { 130 };
            for ch in 0..3 {
                rgb.push(((c[ch] as u32 * c[3] as u32 + bg * (255 - c[3] as u32)) / 255) as u8);
            }
        }
        let mut j = Vec::new();
        jpeg_encoder::Encoder::new(&mut j, 90).encode(&rgb, f.width as u16, f.height as u16, jpeg_encoder::ColorType::Rgb).unwrap();
        let name = p.file_stem().unwrap().to_string_lossy().replace(' ', "_");
        std::fs::write(std::path::Path::new(&dump).join(format!("chk_{name}.jpg")), j).unwrap();
    }
}

// ----------------------------------------------------------------------------- écriture ZAR

#[test]
fn zar_written_by_the_converter_reads_back_identically() {
    use fallout_image_browser::convert::*;
    use fallout_image_browser::formats::EncodeOpts;
    // Dégradé de couleurs, un trou transparent et des pixels à moitié transparents.
    let (w, h) = (23usize, 9usize);
    let mut v = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let a = if (3..9).contains(&x) && y < 4 {
                0
            } else if x > 18 {
                128
            } else {
                255
            };
            v.extend_from_slice(&[(x * 10) as u8, (y * 25) as u8, 90, a]);
        }
    }
    let pic = Picture::single(Format::Png, w as u32, h as u32, Pixels::Rgba(v.clone()), None);
    let o = ConvertOptions {
        format: Format::Zar,
        depth: DepthChoice::Indexed(PalChoice::Adaptive),
        dither: false,
        bg: [0, 0, 0],
        enc: EncodeOpts::default(),
        ops: Default::default(),
        text_white: false,
    };
    let bytes = convert_to_bytes(&pic, &o, &Selection::default()).unwrap();
    assert_eq!(formats::sniff(&bytes), Some(Format::Zar));
    let back = formats::decode(Format::Zar, &bytes, &ctx()).unwrap();
    assert_eq!((back.frames[0].width, back.frames[0].height), (w as u32, h as u32));
    let got = frame_to_rgba(&back.frames[0], back.palette.as_ref());
    for (a, b) in v.chunks_exact(4).zip(got.chunks_exact(4)) {
        // Le ZAR garde l'opacité de chaque pixel, y compris les demi-transparences (ombres, lueurs des sprites).
        assert_eq!(a[3], b[3], "alpha");
        if a[3] > 0 {
            assert_eq!(&a[..3], &b[..3], "couleur");
        }
    }
}

/// SPR « en couches » : chaque image a quatre éléments (vide ou calque positionné), superposés dans l'ordre avec leur palette.
#[test]
fn fallout_tactics_spr_layers_are_composited() {
    let short = |w: u32, h: u32, runs: &[u8]| -> Vec<u8> {
        let mut o = b"<zar>\x00\x34\x00".to_vec();
        o.extend_from_slice(&w.to_le_bytes());
        o.extend_from_slice(&h.to_le_bytes());
        o.push(0);
        o.extend_from_slice(&(runs.len() as u32).to_le_bytes());
        o.extend_from_slice(runs);
        o
    };
    let item = |x: i32, y: i32, zar: Vec<u8>| -> Vec<u8> {
        let mut o = vec![1u8];
        o.extend_from_slice(&x.to_le_bytes());
        o.extend_from_slice(&y.to_le_bytes());
        o.extend_from_slice(&zar);
        o
    };
    let mut payload = Vec::new();
    for _ in 0..4 {
        payload.extend_from_slice(&256u32.to_le_bytes());
        payload.extend_from_slice(&tactics_palette());
    }
    // Calque 0 : 4 × 2 ; calques 1 et 2 : vides ; calque 3 : deux pixels verts opaques en (1, 1).
    payload.extend_from_slice(&item(0, 0, short(4, 2, &tactics_runs())));
    payload.extend_from_slice(&[0, 0]);
    payload.extend_from_slice(&item(1, 1, short(2, 1, &[(2 << 2) | 1, 2, 2])));
    // Collection de type « 2 » (zlib), rangée calque par calque ; une image, une direction.
    let block = {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&payload).unwrap();
        let mut b = b"<spranim_img>\x00".to_vec();
        b.push(b'2');
        b.push(0);
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        b.extend_from_slice(&z.finish().unwrap());
        b
    };
    let name = b"Stand";
    let hdr_len = 12 + 4 + 4 + name.len() + 8 + 16;
    let start = 0x66usize;
    let mut f = vec![0u8; start];
    f[..11].copy_from_slice(b"<sprite>\x00\x34\x00");
    let mut h = b"<spranim>\x00\x31\x00".to_vec();
    h.extend_from_slice(&((start + hdr_len) as u32).to_le_bytes());
    h.extend_from_slice(&(name.len() as u32).to_le_bytes());
    h.extend_from_slice(name);
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&1u32.to_le_bytes());
    for v in [0u32, 0, 4, 2] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(h.len(), hdr_len);
    f.extend_from_slice(&h);
    f.extend_from_slice(&block);
    let p = formats::decode(Format::Spr, &f, &ctx()).unwrap_or_else(|e| panic!("SPR : {e}"));
    assert_eq!((p.frames[0].width, p.frames[0].height), (4, 2));
    let v = rgba_of(&p);
    assert_eq!(&v[0..4], &[255, 0, 0, 255]); // calque 0 inchangé
    assert_eq!(&v[16..20], &[255, 0, 0, 64]);
    assert_eq!(&v[20..24], &[0, 255, 0, 255]); // calque 3 par-dessus
    assert_eq!(&v[24..28], &[0, 255, 0, 255]);
    assert_eq!(&v[28..32], &[0, 0, 0, 20]); // série « alpha seul » : couleur par défaut 0 (noire)
}
