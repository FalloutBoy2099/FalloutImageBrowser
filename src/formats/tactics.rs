//! Fallout Tactics (Micro Forté) : ZAR (images), TIL (tuiles), SPR (sprites animés). Lecture seule.
//!
//! Formats non officiels, reconstitués d'après les travaux de la communauté (outils FORT et FTS) ; les entiers sont
//! petit-boutistes. ZAR : « <zar> », 00 '4' 00, largeur, hauteur, palette 256 × 4 octets, puis des séries RLE.
//! TIL : conteneur « <tile> » contenant des ZAR. SPR : animations nommées (palettes + images ZAR en forme courte).

use super::*;
use std::io::Read;

/// Ordre des canaux dans les entrées de palette de quatre octets : `true` = B, V, R, remplissage (vérifié sur les fichiers du jeu : interface dorée) ; `false` = R, V, B.
const PAL_BGR: bool = true;

fn le32(d: &[u8], o: usize) -> Option<usize> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
}

fn find(d: &[u8], pat: &[u8], from: usize, window: usize) -> Option<usize> {
    let end = d.len().min(from.saturating_add(window).saturating_add(pat.len()));
    if from >= end {
        return None;
    }
    d[from..end].windows(pat.len()).position(|w| w == pat).map(|i| from + i)
}

fn palette_entry(e: &[u8]) -> [u8; 3] {
    if PAL_BGR { [e[2], e[1], e[0]] } else { [e[0], e[1], e[2]] }
}

fn gray_palette() -> Vec<[u8; 3]> {
    (0..256).map(|i| [i as u8; 3]).collect()
}

/// Décodage des séries RLE d'un ZAR : octet de contrôle = nombre (6 bits hauts) et type (2 bits bas).
fn decode_runs(data: &[u8], w: usize, h: usize, pal: &[[u8; 3]], def_color: u8) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let color = |i: u8| pal.get(i as usize).copied().unwrap_or([0, 0, 0]);
    let (mut o, mut p) = (0usize, 0usize);
    let total = w * h;
    while p < data.len() && o < total {
        let c = data[p];
        p += 1;
        let (count, mode) = ((c >> 2) as usize, c & 3);
        for _ in 0..count {
            if o >= total {
                break;
            }
            let px = &mut out[o * 4..o * 4 + 4];
            match mode {
                0 => {} // transparent
                1 => {
                    let Some(&i) = data.get(p) else { return out };
                    p += 1;
                    let rgb = color(i);
                    px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
                2 => {
                    let (Some(&i), Some(&a)) = (data.get(p), data.get(p + 1)) else { return out };
                    p += 2;
                    let rgb = color(i);
                    px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
                }
                _ => {
                    let Some(&a) = data.get(p) else { return out };
                    p += 1;
                    let rgb = color(def_color);
                    px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
                }
            }
            o += 1;
        }
    }
    out
}

struct Zar {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    has_palette: bool,
}

/// ZAR « autonome » commençant à `p` (palette intégrée ou, à défaut, `fallback`).
fn read_zar(d: &[u8], p: usize, fallback: Option<&[[u8; 3]]>) -> Result<Zar> {
    if d.get(p..p + 5) != Some(b"<zar>") {
        bail!("ZAR : signature « <zar> » absente");
    }
    let ty = *d.get(p + 6).ok_or_else(|| anyhow!("ZAR tronqué"))?;
    let (w, h) = (le32(d, p + 8).ok_or_else(|| anyhow!("ZAR tronqué"))?, le32(d, p + 12).ok_or_else(|| anyhow!("ZAR tronqué"))?);
    check_dims(w as u64, h as u64)?;
    let has_pal = *d.get(p + 16).ok_or_else(|| anyhow!("ZAR tronqué"))? != 0;
    let mut q = p + 17;
    let mut pal: Vec<[u8; 3]> = Vec::new();
    if has_pal {
        let n = le32(d, q).ok_or_else(|| anyhow!("ZAR tronqué"))?;
        if n > 256 {
            bail!("ZAR : palette de {n} entrées");
        }
        q += 4;
        let raw = d.get(q..q + n * 4).ok_or_else(|| anyhow!("ZAR : palette tronquée"))?;
        pal = raw.chunks_exact(4).map(palette_entry).collect();
        q += n * 4;
    } else if let Some(f) = fallback {
        pal = f.to_vec();
    }
    if pal.is_empty() {
        pal = gray_palette();
    }
    let def_color = if ty == 0x34 {
        let v = *d.get(q).ok_or_else(|| anyhow!("ZAR tronqué"))?;
        q += 1;
        v
    } else {
        0
    };
    let size = le32(d, q).ok_or_else(|| anyhow!("ZAR tronqué"))?;
    q += 4;
    let data = d.get(q..(q + size).min(d.len())).ok_or_else(|| anyhow!("ZAR tronqué"))?;
    Ok(Zar { w, h, rgba: decode_runs(data, w, h, &pal, def_color), has_palette: has_pal })
}

pub fn decode_zar(data: &[u8]) -> Result<Picture> {
    let z = read_zar(data, 0, None)?;
    let mut pic = Picture::single(Format::Zar, z.w as u32, z.h as u32, Pixels::Rgba(z.rgba), None);
    pic.bpp = 32;
    let s = pic.section("sec.format");
    s.add("p.compression", "RLE");
    s.add("p.palette_entries", if z.has_palette { 256 } else { 0 });
    Ok(pic)
}

/// ZAR en forme courte (couleur par défaut, taille, données) commençant à `z` ; renvoie (largeur, hauteur, RVBA, fin).
fn read_short_zar(d: &[u8], z: usize, pal: &[[u8; 3]]) -> Result<(usize, usize, Vec<u8>, usize)> {
    let (w, h) = (le32(d, z + 8).ok_or_else(|| anyhow!("ZAR tronqué"))?, le32(d, z + 12).ok_or_else(|| anyhow!("ZAR tronqué"))?);
    check_dims(w as u64, h as u64)?;
    let def = *d.get(z + 16).ok_or_else(|| anyhow!("ZAR tronqué"))?;
    let size = le32(d, z + 17).ok_or_else(|| anyhow!("ZAR tronqué"))?;
    let run = d.get(z + 21..(z + 21 + size).min(d.len())).ok_or_else(|| anyhow!("ZAR tronqué"))?;
    Ok((w, h, decode_runs(run, w, h, pal, def), z + 21 + size))
}

pub fn decode_til(data: &[u8]) -> Result<Picture> {
    if data.get(..6) != Some(b"<tile>") {
        bail!("TIL : signature « <tile> » absente");
    }
    // Largeur et hauteur : après la chaîne terminée par 0, puis 7 + 4 octets inconnus.
    let mut q = 7usize;
    while data.get(q).is_some_and(|b| *b != 0) {
        q += 1;
    }
    q += 1 + 7 + 4;
    let dims = (le32(data, q), le32(data, q + 4));
    // Premier ZAR : après « <tiledata> » (+ 3 octets et le nombre d'images), à défaut à l'offset 48.
    let (first, count_hint) = match find(data, b"<tiledata>", 0, data.len()) {
        Some(m) => (find(data, b"<zar>", m, 256), le32(data, m + 13)),
        None => (find(data, b"<zar>", 48, 64), None),
    };
    let mut at = first.ok_or_else(|| anyhow!("TIL : aucun ZAR trouvé"))?;
    // Les ZAR d'un TIL sont en forme courte (sans palette) ; la palette (256 entrées de 4 octets) termine le fichier.
    let pal: Vec<[u8; 3]> = match data.len().checked_sub(1028).and_then(|p| Some((le32(data, p)?, data.get(p + 4..)?))) {
        Some((256, raw)) => raw.chunks_exact(4).map(palette_entry).collect(),
        _ => gray_palette(),
    };
    let mut pic = Picture::new(Format::Til);
    let want = count_hint.filter(|c| (1..=4096).contains(c)).unwrap_or(1);
    for _ in 0..want {
        let (w, h, rgba, end) = read_short_zar(data, at, &pal)?;
        pic.frames.push(Frame { width: w as u32, height: h as u32, pixels: Pixels::Rgba(rgba), pos: (0, 0), delay_ms: Some(100) });
        match find(data, b"<zar>", end, 64) {
            Some(n) => at = n,
            None => break,
        }
    }
    let n = pic.frames.len();
    pic.frames_per_dir = n;
    pic.fps = 10;
    pic.bpp = 32;
    let s = pic.section("sec.format");
    s.add("p.frames", n);
    if let (Some(w), Some(h)) = dims {
        s.add("p.dimensions", format!("{w} × {h}"));
    }
    Ok(pic)
}

struct Anim {
    name: String,
    data_offset: usize,
    frames: usize,
    dirs: usize,
    rects: Vec<[i64; 4]>,
}

pub fn decode_spr(data: &[u8]) -> Result<Picture> {
    if data.get(..8) != Some(b"<sprite>") {
        bail!("SPR : signature « <sprite> » absente");
    }
    let anchor = (le32(data, 14).unwrap_or(0) as i64, le32(data, 18).unwrap_or(0) as i64);
    let first = find(data, b"<spranim>", 8, 64 * 1024).ok_or_else(|| anyhow!("SPR : aucune animation"))?;
    // Chaîne d'en-têtes d'animation.
    let mut anims: Vec<Anim> = Vec::new();
    let mut at = first;
    while data.get(at..at + 9) == Some(b"<spranim>") && anims.len() < 4096 {
        let data_offset = le32(data, at + 12).ok_or_else(|| anyhow!("SPR tronqué"))? + 0x0C;
        let nl = le32(data, at + 16).ok_or_else(|| anyhow!("SPR tronqué"))?;
        let name = String::from_utf8_lossy(data.get(at + 20..at + 20 + nl).ok_or_else(|| anyhow!("SPR tronqué"))?).to_string();
        let after = at + 20 + nl;
        let (frames, dirs) = (le32(data, after).ok_or_else(|| anyhow!("SPR tronqué"))?, le32(data, after + 4).ok_or_else(|| anyhow!("SPR tronqué"))?);
        let n = frames.checked_mul(dirs).filter(|n| *n <= 100_000).ok_or_else(|| anyhow!("SPR : nombre d'images invalide"))?;
        let mut rects = Vec::with_capacity(n);
        for i in 0..n {
            let o = after + 8 + i * 16;
            let r = [le32(data, o), le32(data, o + 4), le32(data, o + 8), le32(data, o + 12)];
            let Some(v) = r.iter().map(|x| x.map(|x| x as i64)).collect::<Option<Vec<i64>>>() else { bail!("SPR tronqué") };
            rects.push([v[0], v[1], v[2], v[3]]);
        }
        anims.push(Anim { name, data_offset, frames, dirs, rects });
        at = after + 8 + n * 16;
    }
    // Début de chaque bloc de données (pour borner le flux zlib).
    let mut starts: Vec<usize> = anims.iter().map(|a| a.data_offset).collect();
    starts.sort_unstable();
    // Toutes les animations deviennent des « directions » libellées ; les plus courtes sont complétées par des images vides.
    let max_frames = anims.iter().map(|a| a.frames).max().unwrap_or(1).max(1);
    let mut pic = Picture::new(Format::Spr);
    let mut total_frames = 0usize;
    for anim in anims.iter().filter(|a| a.frames * a.dirs > 0) {
        total_frames += anim.frames * anim.dirs;
        if total_frames > 40_000 {
            break;
        }
        let end = starts.iter().copied().find(|s| *s > anim.data_offset).unwrap_or(data.len());
        let frames = decode_animation(data, anim, end, anchor)?;
        for d in 0..anim.dirs {
            let mut dir_frames: Vec<Frame> = frames[d * anim.frames..(d + 1) * anim.frames].to_vec();
            let last_pos = dir_frames.last().map(|f| f.pos).unwrap_or((0, 0));
            while dir_frames.len() < max_frames {
                dir_frames.push(Frame { width: 1, height: 1, pixels: Pixels::Rgba(vec![0, 0, 0, 0]), pos: last_pos, delay_ms: Some(100) });
            }
            pic.frames.extend(dir_frames);
            pic.dir_labels.push(if anim.dirs > 1 { format!("{} — {}/{}", anim.name, d + 1, anim.dirs) } else { anim.name.clone() });
            pic.dir_counts.push(anim.frames);
        }
    }
    if pic.frames.is_empty() {
        bail!("SPR : animations vides");
    }
    pic.directions = pic.dir_labels.len();
    pic.frames_per_dir = max_frames;
    pic.dir_shift = vec![(0, 0); pic.directions];
    pic.fps = 10;
    pic.bpp = 32;
    let s = pic.section("sec.format");
    s.raw("Animations in file", anims.len());
    s.raw("Anchor", format!("{}, {}", anchor.0, anchor.1));
    let list = pic.section("sec.frames");
    for (i, a) in anims.iter().take(80).enumerate() {
        list.raw(&format!("#{i}"), format!("{}  ({} × {})", a.name, a.dirs, a.frames));
    }
    Ok(pic)
}

/// Images (direction par direction) d'une animation de SPR.
fn decode_animation(data: &[u8], anim: &Anim, end: usize, anchor: (i64, i64)) -> Result<Vec<Frame>> {
    let version = *data.get(anim.data_offset + 2).ok_or_else(|| anyhow!("SPR tronqué"))?;
    let payload: Vec<u8> = match version {
        b'1' => data.get(anim.data_offset + 4..end).ok_or_else(|| anyhow!("SPR tronqué"))?.to_vec(),
        b'2' => {
            let expected = le32(data, anim.data_offset + 4).ok_or_else(|| anyhow!("SPR tronqué"))?;
            let mut v = Vec::with_capacity(expected.min(1 << 28));
            flate2::read::ZlibDecoder::new(data.get(anim.data_offset + 8..end).ok_or_else(|| anyhow!("SPR tronqué"))?)
                .read_to_end(&mut v)
                .map_err(|e| anyhow!("SPR zlib : {e}"))?;
            v
        }
        v => bail!("SPR : version de bloc inconnue ({:?})", v as char),
    };
    // Quatre palettes (nombre + entrées de quatre octets) ; la première est la palette en couleurs.
    let mut p = 0usize;
    let mut palettes: Vec<Vec<[u8; 3]>> = Vec::new();
    for _ in 0..4 {
        let n = le32(&payload, p).ok_or_else(|| anyhow!("SPR : palettes tronquées"))?;
        if n > 256 {
            bail!("SPR : palette de {n} entrées");
        }
        let raw = payload.get(p + 4..p + 4 + n * 4).ok_or_else(|| anyhow!("SPR : palettes tronquées"))?;
        palettes.push(raw.chunks_exact(4).map(palette_entry).collect());
        p += 4 + n * 4;
    }
    let pal = &palettes[0];
    let total = anim.frames * anim.dirs;
    let mut out = Vec::with_capacity(total);
    let mut cursor = p;
    for i in 0..total {
        // Les images sont repérées par leur signature (écarts de 9, 10 ou 12 octets selon les fichiers).
        let frame = match find(&payload, b"<zar>", cursor, 64) {
            Some(z) => {
                let (w, h) = (le32(&payload, z + 8).unwrap_or(0), le32(&payload, z + 12).unwrap_or(0));
                let def = payload.get(z + 16).copied().unwrap_or(0);
                let size = le32(&payload, z + 17).unwrap_or(0);
                cursor = z + 21 + size;
                if w * h > 64_000_000 {
                    bail!("SPR : image trop grande");
                }
                let rgba = if w == 0 || h == 0 {
                    vec![0, 0, 0, 0]
                } else {
                    let d = payload.get(z + 21..(z + 21 + size).min(payload.len())).unwrap_or(&[]);
                    decode_runs(d, w, h, pal, def)
                };
                let (w1, h1) = if w == 0 || h == 0 { (1, 1) } else { (w, h) };
                let r = anim.rects.get(i).copied().unwrap_or([0; 4]);
                let pos = if r == [0; 4] { (0, 0) } else { ((r[0] - anchor.0) as i32, (r[1] - anchor.1) as i32) };
                Frame { width: w1 as u32, height: h1 as u32, pixels: Pixels::Rgba(rgba), pos, delay_ms: Some(100) }
            }
            // Animation qui déclare plus d'images qu'elle n'en contient : image vide.
            None => Frame { width: 1, height: 1, pixels: Pixels::Rgba(vec![0, 0, 0, 0]), pos: (0, 0), delay_ms: Some(100) },
        };
        out.push(frame);
    }
    Ok(out)
}
