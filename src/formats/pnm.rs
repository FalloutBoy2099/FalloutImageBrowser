//! PNM : PBM / PGM / PPM (P1 à P6, ASCII et binaire) et PAM (P7). Écriture : P6 (24 bits) et P7 RGB_ALPHA (32 bits).

use super::*;

struct Tok<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Tok<'a> {
    fn skip_ws(&mut self) {
        while self.p < self.d.len() {
            match self.d[self.p] {
                b'#' => {
                    while self.p < self.d.len() && self.d[self.p] != b'\n' {
                        self.p += 1;
                    }
                }
                c if c.is_ascii_whitespace() => self.p += 1,
                _ => break,
            }
        }
    }
    fn word(&mut self) -> Option<&'a str> {
        self.skip_ws();
        let s = self.p;
        while self.p < self.d.len() && !self.d[self.p].is_ascii_whitespace() {
            self.p += 1;
        }
        if s == self.p { None } else { std::str::from_utf8(&self.d[s..self.p]).ok() }
    }
    fn num(&mut self) -> Result<usize> {
        self.word().and_then(|w| w.parse().ok()).ok_or_else(|| anyhow!("PNM : en-tête invalide"))
    }
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    if data.len() < 3 || data[0] != b'P' || !(b'1'..=b'7').contains(&data[1]) {
        bail!("signature PNM absente");
    }
    let kind = data[1] - b'0';
    let mut t = Tok { d: data, p: 2 };
    let (w, h, maxval, depth, tupl): (usize, usize, usize, usize, String);
    if kind == 7 {
        // PAM : lignes « MOT valeur » jusqu'à ENDHDR.
        let (mut ww, mut hh, mut dd, mut mv, mut tt) = (0, 0, 0, 255usize, String::new());
        loop {
            let key = t.word().ok_or_else(|| anyhow!("PAM : en-tête tronqué"))?;
            match key {
                "ENDHDR" => break,
                "WIDTH" => ww = t.num()?,
                "HEIGHT" => hh = t.num()?,
                "DEPTH" => dd = t.num()?,
                "MAXVAL" => mv = t.num()?,
                "TUPLTYPE" => {
                    // jusqu'à la fin de la ligne
                    t.skip_ws();
                    let s = t.p;
                    while t.p < data.len() && data[t.p] != b'\n' {
                        t.p += 1;
                    }
                    tt = String::from_utf8_lossy(&data[s..t.p]).trim().to_string();
                }
                _ => {
                    let _ = t.word();
                }
            }
        }
        (w, h, maxval, depth, tupl) = (ww, hh, mv, dd, tt);
        t.p += 1; // l'unique caractère blanc qui suit ENDHDR
    } else {
        w = t.num()?;
        h = t.num()?;
        maxval = if kind == 1 || kind == 4 { 1 } else { t.num()? };
        depth = if kind == 3 || kind == 6 { 3 } else { 1 };
        tupl = String::new();
        if kind != 1 && kind != 2 && kind != 3 {
            t.p += 1; // un seul blanc entre l'en-tête et les données binaires
        }
    }
    check_dims(w as u64, h as u64)?;
    if maxval == 0 || maxval > 65535 || depth == 0 || depth > 4 {
        bail!("PNM : paramètres invalides");
    }
    let n = w * h;
    let two = maxval > 255;
    let scale = |v: usize| -> u8 { (v.min(maxval) * 255 / maxval) as u8 };
    // Échantillons (n × depth) en entiers.
    let mut samples = vec![0usize; n * depth];
    match kind {
        1..=3 => {
            for s in samples.iter_mut() {
                let word = if kind == 1 {
                    // PBM ASCII : les chiffres peuvent être collés.
                    t.skip_ws();
                    let c = *data.get(t.p).ok_or_else(|| anyhow!("PNM : données tronquées"))?;
                    t.p += 1;
                    (c - b'0') as usize
                } else {
                    t.num()?
                };
                *s = word;
            }
        }
        4 => {
            let rb = w.div_ceil(8);
            let body = data.get(t.p..t.p + rb * h).ok_or_else(|| anyhow!("PNM : données tronquées"))?;
            for y in 0..h {
                for x in 0..w {
                    samples[y * w + x] = ((body[y * rb + x / 8] >> (7 - x % 8)) & 1) as usize;
                }
            }
        }
        _ => {
            let bps = if two { 2 } else { 1 };
            let body = data.get(t.p..t.p + n * depth * bps).ok_or_else(|| anyhow!("PNM : données tronquées"))?;
            for (i, s) in samples.iter_mut().enumerate() {
                *s = if two { u16::from_be_bytes([body[i * 2], body[i * 2 + 1]]) as usize } else { body[i] as usize };
            }
        }
    }
    let mut out = vec![255u8; n * 4];
    let pbm = kind == 1 || kind == 4 || tupl == "BLACKANDWHITE";
    for i in 0..n {
        let s = &samples[i * depth..i * depth + depth];
        let o = i * 4;
        if pbm {
            // PBM : 1 = noir ; PAM BLACKANDWHITE : 1 = blanc.
            let v = if kind == 7 { s[0] * 255 } else { (1 - s[0].min(1)) * 255 } as u8;
            out[o..o + 3].fill(v);
        } else if depth >= 3 {
            out[o] = scale(s[0]);
            out[o + 1] = scale(s[1]);
            out[o + 2] = scale(s[2]);
            if depth == 4 {
                out[o + 3] = scale(s[3]);
            }
        } else {
            let v = scale(s[0]);
            out[o..o + 3].fill(v);
            if depth == 2 {
                out[o + 3] = scale(s[1]);
            }
        }
    }
    let mut pic = Picture::single(Format::Pnm, w as u32, h as u32, Pixels::Rgba(out), None);
    pic.bpp = (depth * if two { 16 } else { 8 }) as u32;
    if pbm {
        pic.bpp = 1;
    }
    let s = pic.section("sec.format");
    s.raw(
        "Type",
        format!(
            "P{kind}{}",
            match kind {
                1 | 4 => " (PBM)",
                2 | 5 => " (PGM)",
                3 | 6 => " (PPM)",
                _ => " (PAM)",
            }
        ),
    );
    s.add("p.compression", if matches!(kind, 1..=3) { "ASCII" } else { "p.none" });
    s.raw("Maxval", maxval);
    if !tupl.is_empty() {
        s.raw("Tuple type", tupl);
    }
    Ok(pic)
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let OutData::Rgba(px) = &f.data else { bail!("PNM : image RVB requise") };
    let mut o: Vec<u8> = Vec::new();
    if p.depth == Depth::Rgba32 {
        o.extend_from_slice(format!("P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n", f.width, f.height).as_bytes());
        o.extend_from_slice(px);
    } else {
        o.extend_from_slice(format!("P6\n{} {}\n255\n", f.width, f.height).as_bytes());
        for c in px.chunks_exact(4) {
            o.extend_from_slice(&c[..3]);
        }
    }
    Ok(o)
}
