//! Fallout FRM (1 & 2) : images 8 bits indexées, jusqu'à 6 directions, big-endian.
//! https://fallout.wiki/wiki/FRM_File_Format

use super::*;

const HEADER: usize = 0x3E;

pub fn decode(data: &[u8], ctx: &DecodeCtx) -> Result<Picture> {
    let mut r = Rd::new(data);
    let version = r.u32be()?;
    let fps = r.u16be()?;
    let action = r.u16be()?;
    let fpd = r.u16be()? as usize;
    let mut sx = [0i16; 6];
    let mut sy = [0i16; 6];
    let mut offs = [0u32; 6];
    for v in sx.iter_mut() {
        *v = r.i16be()?;
    }
    for v in sy.iter_mut() {
        *v = r.i16be()?;
    }
    for v in offs.iter_mut() {
        *v = r.u32be()?;
    }
    let total = r.u32be()?;
    if fpd == 0 {
        bail!("FRM sans image (0 image par direction)");
    }
    let dirs = (1..6).rev().find(|&d| offs[d] != 0).map(|d| d + 1).unwrap_or(1);

    let mut pic = Picture::new(Format::Frm);
    pic.directions = dirs;
    pic.frames_per_dir = fpd;
    pic.fps = if fps == 0 { 10 } else { fps as u32 };
    pic.action_frame = action as u32;
    pic.dir_shift = (0..dirs).map(|d| (sx[d] as i32, sy[d] as i32)).collect();
    pic.palette = Some((*ctx.palette).clone());
    pic.bpp = 8;
    let delay = (1000 / pic.fps.max(1)) as u32;
    let mut frame_rows: Vec<String> = Vec::new();
    for d in 0..dirs {
        let mut pos = HEADER + offs[d] as usize;
        let (mut cx, mut cy) = (sx[d] as i32, sy[d] as i32);
        for _f in 0..fpd {
            let mut fr = Rd::at(data, pos);
            let w = fr.u16be()? as usize;
            let h = fr.u16be()? as usize;
            let size = fr.u32be()? as usize;
            let dx = fr.i16be()? as i32;
            let dy = fr.i16be()? as i32;
            let npx = w * h;
            let px = fr.bytes(npx)?;
            cx += dx;
            cy += dy;
            let (w1, h1, idx) = if npx == 0 { (1usize, 1usize, vec![0u8]) } else { (w, h, px.to_vec()) };
            pic.frames.push(Frame {
                width: w1 as u32,
                height: h1 as u32,
                pixels: Pixels::Indexed(idx),
                pos: (cx - (w1 as i32) / 2, cy - h1 as i32),
                delay_ms: Some(delay),
            });
            if d == 0 && frame_rows.len() < 64 {
                frame_rows.push(format!("{w}×{h}  ({dx:+}, {dy:+})"));
            }
            pos += 12 + size.max(npx);
        }
    }
    let s = pic.section("sec.format");
    s.add("p.version", version);
    s.add("p.fps", fps);
    s.add("p.action_frame", action);
    s.add("p.frames_per_dir", fpd);
    s.add("p.directions", dirs);
    s.add("p.data_size", total);
    s.add("p.palette_src", ctx.palette_name.clone());
    for d in 0..dirs {
        s.raw(&format!("Dir {d} shift"), format!("{}, {}", sx[d], sy[d]));
    }
    let s = pic.section("sec.frames");
    for (i, row) in frame_rows.iter().enumerate() {
        s.raw(&format!("#{i}"), row);
    }
    Ok(pic)
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let pal_ok = p.palette.is_some();
    if !pal_ok {
        bail!("FRM : palette manquante");
    }
    let dirs = p.directions.clamp(1, 6);
    let fpd = p.frames_per_dir;
    if fpd == 0 || fpd > 65535 {
        bail!("nombre d'images invalide");
    }
    let mut body: Vec<u8> = Vec::new();
    let mut offs = [0u32; 6];
    let mut sx = [0i16; 6];
    let mut sy = [0i16; 6];
    for d in 0..dirs {
        offs[d] = if d == 0 { 0 } else { body.len() as u32 };
        let (shx, shy) = p.dir_shift.get(d).copied().unwrap_or((0, 0));
        sx[d] = shx as i16;
        sy[d] = shy as i16;
        let (mut pcx, mut pcy) = (shx, shy);
        for f in 0..fpd {
            let fr = &p.frames[d * fpd + f];
            let idx = match &fr.data {
                OutData::Indexed(v) => v,
                _ => bail!("FRM : image non indexée"),
            };
            if fr.width > 65535 || fr.height > 65535 {
                bail!("image trop grande pour le format FRM");
            }
            let cx = fr.pos.0 + fr.width as i32 / 2;
            let cy = fr.pos.1 + fr.height as i32;
            let (dx, dy) = (cx - pcx, cy - pcy);
            pcx = cx;
            pcy = cy;
            put_u16be(&mut body, fr.width as u16);
            put_u16be(&mut body, fr.height as u16);
            put_u32be(&mut body, idx.len() as u32);
            put_u16be(&mut body, dx.clamp(-32768, 32767) as i16 as u16);
            put_u16be(&mut body, dy.clamp(-32768, 32767) as i16 as u16);
            body.extend_from_slice(idx);
        }
    }
    let mut o: Vec<u8> = Vec::with_capacity(HEADER + body.len());
    put_u32be(&mut o, 4);
    put_u16be(&mut o, p.fps.clamp(1, 65535) as u16);
    put_u16be(&mut o, p.action_frame.min(fpd as u32 - 1) as u16);
    put_u16be(&mut o, fpd as u16);
    for v in sx {
        put_u16be(&mut o, v as u16);
    }
    for v in sy {
        put_u16be(&mut o, v as u16);
    }
    for v in offs {
        put_u32be(&mut o, v);
    }
    put_u32be(&mut o, body.len() as u32);
    o.extend_from_slice(&body);
    Ok(o)
}
