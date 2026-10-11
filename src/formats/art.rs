//! ART (Arcanum / Troika), lignes de haut en bas : palettes BGRA, en-têtes d'image de 28 octets, RLE, little-endian.
//! https://github.com/Bia10/ArcNET/blob/main/src/Formats/ArcNET.Formats/ArtFormat.cs

use super::*;

const STATIC: u32 = 0x1;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut r = Rd::new(data);
    let flags = r.u32le()?;
    let rate = r.u32le()?;
    let _rot = r.u32le()?;
    let mut pal_ids = [0i32; 4];
    for v in pal_ids.iter_mut() {
        *v = r.i32le()?;
    }
    let action = r.u32le()?;
    let fcount = r.u32le()? as usize;
    r.skip(32)?; // PaletteData1
    let mut dsizes = [0u32; 8];
    for v in dsizes.iter_mut() {
        *v = r.u32le()?;
    }
    r.skip(32)?; // PaletteData2
    let rots = if flags & STATIC != 0 { 1 } else { 8 };
    if fcount == 0 || fcount > 4096 {
        bail!("nombre d'images ART invalide ({fcount})");
    }
    let mut palettes: Vec<Palette> = Vec::new();
    for slot in 0..4 {
        if pal_ids[slot] == 0 {
            continue;
        }
        let raw = r.bytes(1024)?;
        let mut p = Palette::empty();
        for i in 0..256 {
            p.colors[i] = [raw[i * 4 + 2], raw[i * 4 + 1], raw[i * 4], 255];
            p.used[i] = true;
        }
        p.colors[0][3] = 0;
        palettes.push(p);
    }
    let palette = palettes.first().cloned().unwrap_or_else(|| {
        let mut p = Palette::empty();
        for i in 0..256 {
            p.colors[i] = [i as u8, i as u8, i as u8, 255];
            p.used[i] = true;
        }
        p.colors[0][3] = 0;
        p
    });
    struct Hdr {
        w: usize,
        h: usize,
        size: usize,
        cx: i32,
        cy: i32,
        dx: i32,
        dy: i32,
    }
    let total = rots * fcount;
    let mut hdrs = Vec::with_capacity(total);
    for _ in 0..total {
        hdrs.push(Hdr {
            w: r.u32le()? as usize,
            h: r.u32le()? as usize,
            size: r.u32le()? as usize,
            cx: r.i32le()?,
            cy: r.i32le()?,
            dx: r.i32le()?,
            dy: r.i32le()?,
        });
    }
    let mut pic = Picture::new(Format::Art);
    pic.directions = rots;
    pic.frames_per_dir = fcount;
    pic.fps = rate.clamp(1, 1000);
    pic.action_frame = action;
    pic.dir_shift = vec![(0, 0); rots];
    pic.palette = Some(palette);
    pic.bpp = 8;
    let delay = 1000 / pic.fps;
    for h in &hdrs {
        check_dims(h.w.max(1) as u64, h.h.max(1) as u64)?;
        let block = r.bytes(h.size)?;
        let n = h.w * h.h;
        let mut px = vec![0u8; n];
        if h.size == n {
            px.copy_from_slice(block);
        } else {
            let (mut i, mut o) = (0usize, 0usize);
            while o < n && i < block.len() {
                let c = block[i];
                i += 1;
                let cnt = ((c & 0x7F) as usize).min(n - o);
                if c & 0x80 == 0 {
                    let v = *block.get(i).unwrap_or(&0);
                    i += 1;
                    px[o..o + cnt].fill(v);
                } else {
                    let end = (i + cnt).min(block.len());
                    px[o..o + (end - i)].copy_from_slice(&block[i..end]);
                    i += cnt;
                }
                o += cnt;
            }
        }
        // Les lignes sont stockées de haut en bas (constaté sur de vrais fichiers).
        let top = px;
        let (w1, h1, top) = if n == 0 { (1, 1, vec![0u8]) } else { (h.w, h.h, top) };
        pic.frames.push(Frame {
            width: w1 as u32,
            height: h1 as u32,
            pixels: Pixels::Indexed(top),
            pos: (-h.cx, -h.cy),
            delay_ms: Some(delay),
        });
    }
    let s = pic.section("sec.format");
    s.raw("Flags", format!("{flags:#010X}{}", if flags & STATIC != 0 { " (static)" } else { "" }));
    s.add("p.fps", rate);
    s.add("p.action_frame", action);
    s.add("p.frames_per_dir", fcount);
    s.add("p.directions", rots);
    s.raw("Palettes", palettes.len());
    s.raw("Palette IDs", format!("{:?}", pal_ids));
    let s = pic.section("sec.frames");
    for (i, h) in hdrs.iter().take(64).enumerate() {
        s.raw(
            &format!("#{i}"),
            format!("{}×{}  center ({}, {})  delta ({}, {})  {} B", h.w, h.h, h.cx, h.cy, h.dx, h.dy, h.size),
        );
    }
    Ok(pic)
}

fn rle(px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut lit_start = 0usize;
    let flush = |out: &mut Vec<u8>, from: usize, to: usize| {
        let mut s = from;
        while s < to {
            let n = (to - s).min(127);
            out.push(0x80 | n as u8);
            out.extend_from_slice(&px[s..s + n]);
            s += n;
        }
    };
    while i < px.len() {
        let v = px[i];
        let mut n = 1;
        while i + n < px.len() && px[i + n] == v && n < 127 {
            n += 1;
        }
        if n >= 3 {
            flush(&mut out, lit_start, i);
            out.push(n as u8);
            out.push(v);
            i += n;
            lit_start = i;
        } else {
            i += n;
        }
    }
    flush(&mut out, lit_start, px.len());
    out
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let Some(pal) = &p.palette else { bail!("ART : palette manquante") };
    let rots = if p.directions <= 1 { 1 } else { 8 };
    let fpd = p.frames_per_dir;
    let mut blocks: Vec<(u32, u32, Vec<u8>, i32, i32)> = Vec::new();
    let mut dsizes = [0u32; 8];
    for r in 0..rots {
        let src_dir = r % p.directions.max(1);
        let mut dir_total = 0usize;
        for f in 0..fpd {
            let fr = &p.frames[src_dir * fpd + f];
            let OutData::Indexed(idx) = &fr.data else { bail!("ART : image indexée requise") };
            let bottom_up = idx.clone();
            let enc = rle(&bottom_up);
            let data = if enc.len() >= bottom_up.len() { bottom_up } else { enc };
            dir_total += data.len();
            blocks.push((fr.width, fr.height, data, -fr.pos.0, -fr.pos.1));
        }
        dsizes[r] = dir_total as u32;
    }
    let mut o: Vec<u8> = Vec::new();
    put_u32le(&mut o, if rots == 1 { STATIC } else { 0 });
    put_u32le(&mut o, p.fps.clamp(1, 1000));
    put_u32le(&mut o, 8);
    for id in [1u32, 0, 0, 0] {
        put_u32le(&mut o, id);
    }
    put_u32le(&mut o, p.action_frame);
    put_u32le(&mut o, fpd as u32);
    o.extend_from_slice(&[0u8; 32]);
    for d in dsizes {
        put_u32le(&mut o, d);
    }
    o.extend_from_slice(&[0u8; 32]);
    for i in 0..256 {
        let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
        o.extend_from_slice(&[c[2], c[1], c[0], 255]);
    }
    for (w, h, data, cx, cy) in &blocks {
        put_u32le(&mut o, *w);
        put_u32le(&mut o, *h);
        put_u32le(&mut o, data.len() as u32);
        put_u32le(&mut o, *cx as u32);
        put_u32le(&mut o, *cy as u32);
        put_u32le(&mut o, 0);
        put_u32le(&mut o, 0);
    }
    for (_, _, data, _, _) in &blocks {
        o.extend_from_slice(data);
    }
    Ok(o)
}
