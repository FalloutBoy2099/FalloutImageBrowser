//! GIF 87a / 89a, animé ou non.

use super::*;
use ::gif as gifc;
use std::io::Cursor;

struct Raw {
    left: usize,
    top: usize,
    w: usize,
    h: usize,
    idx: Vec<u8>,
    pal: Vec<u8>,
    transparent: Option<u8>,
    delay: u32,
    dispose: gifc::DisposalMethod,
    interlaced: bool,
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut opts = gifc::DecodeOptions::new();
    opts.set_color_output(gifc::ColorOutput::Indexed);
    let mut dec = opts.read_info(Cursor::new(data)).map_err(|e| anyhow!("GIF : {e}"))?;
    let (cw, ch) = (dec.width() as usize, dec.height() as usize);
    check_dims(cw as u64, ch as u64)?;
    let global: Vec<u8> = dec.global_palette().map(|p| p.to_vec()).unwrap_or_default();
    let bg_index = dec.bg_color();
    let mut raws: Vec<Raw> = Vec::new();
    while let Some(f) = dec.read_next_frame().map_err(|e| anyhow!("GIF : {e}"))? {
        raws.push(Raw {
            left: f.left as usize,
            top: f.top as usize,
            w: f.width as usize,
            h: f.height as usize,
            idx: f.buffer.to_vec(),
            pal: f.palette.clone().unwrap_or_else(|| global.clone()),
            transparent: f.transparent,
            delay: f.delay as u32 * 10,
            dispose: f.dispose,
            interlaced: f.interlaced,
        });
        if raws.len() > 20000 {
            break;
        }
    }
    if raws.is_empty() {
        bail!("GIF sans image");
    }
    let make_pal = |r: &Raw| {
        let mut p = Palette::empty();
        for i in 0..(r.pal.len() / 3).min(256) {
            p.colors[i] = [r.pal[i * 3], r.pal[i * 3 + 1], r.pal[i * 3 + 2], 255];
            p.used[i] = true;
        }
        if let Some(t) = r.transparent {
            p.colors[t as usize][3] = 0;
            p.used[t as usize] = true;
        }
        p
    };
    let mut pic;
    let n = raws.len();
    let first = &raws[0];
    if n == 1 && first.left == 0 && first.top == 0 && first.w == cw && first.h == ch {
        let pal = make_pal(first);
        pic = Picture::single(Format::Gif, cw as u32, ch as u32, Pixels::Indexed(first.idx.clone()), Some(pal));
        pic.frames[0].delay_ms = Some(first.delay);
    } else {
        pic = Picture::new(Format::Gif);
        let mut canvas = vec![0u8; cw * ch * 4];
        for r in &raws {
            let saved = if r.dispose == gifc::DisposalMethod::Previous { Some(canvas.clone()) } else { None };
            for y in 0..r.h {
                let cy = r.top + y;
                if cy >= ch {
                    break;
                }
                for x in 0..r.w {
                    let cx = r.left + x;
                    if cx >= cw {
                        break;
                    }
                    let i = r.idx[y * r.w + x];
                    if Some(i) == r.transparent {
                        continue;
                    }
                    let p = (i as usize) * 3;
                    if p + 2 < r.pal.len() {
                        let o = (cy * cw + cx) * 4;
                        canvas[o..o + 4].copy_from_slice(&[r.pal[p], r.pal[p + 1], r.pal[p + 2], 255]);
                    }
                }
            }
            check_anim_budget(pic.frames.len() + 1, cw * ch * 4)?;
            pic.frames.push(Frame {
                width: cw as u32,
                height: ch as u32,
                pixels: Pixels::Rgba(canvas.clone()),
                pos: (0, 0),
                delay_ms: Some(if r.delay < 20 { 100 } else { r.delay }),
            });
            match r.dispose {
                gifc::DisposalMethod::Background => {
                    for y in r.top..(r.top + r.h).min(ch) {
                        for x in r.left..(r.left + r.w).min(cw) {
                            canvas[(y * cw + x) * 4..(y * cw + x) * 4 + 4].fill(0);
                        }
                    }
                }
                gifc::DisposalMethod::Previous => {
                    if let Some(s) = saved {
                        canvas = s;
                    }
                }
                _ => {}
            }
        }
        pic.frames_per_dir = n;
        let total: u32 = pic.frames.iter().map(|f| f.delay_ms.unwrap_or(100)).sum();
        pic.fps = ((n as u32 * 1000) / total.max(1)).max(1);
    }
    pic.bpp = 8;
    let version = String::from_utf8_lossy(&data[..6.min(data.len())]).to_string();
    let s = pic.section("sec.format");
    s.raw("Version", version);
    s.add("p.frames", n);
    s.add("p.palette_entries", global.len() / 3);
    s.raw("Background index", bg_index.map(|v| v.to_string()).unwrap_or_default());
    s.add("p.interlaced", if raws.iter().any(|r| r.interlaced) { "p.yes" } else { "p.no" });
    s.add("p.compression", "LZW");
    if n > 1 {
        let total: u32 = raws.iter().map(|r| r.delay).sum();
        s.add("p.duration", format!("{:.2} s", total as f64 / 1000.0));
    }
    Ok(pic)
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let Some(pal) = &p.palette else { bail!("GIF : palette manquante") };
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for f in &p.frames {
        x0 = x0.min(f.pos.0);
        y0 = y0.min(f.pos.1);
        x1 = x1.max(f.pos.0 + f.width as i32);
        y1 = y1.max(f.pos.1 + f.height as i32);
    }
    let (cw, ch) = ((x1 - x0) as u32, (y1 - y0) as u32);
    if cw > 65535 || ch > 65535 {
        bail!("image trop grande pour le format GIF");
    }
    let mut gp = Vec::with_capacity(768);
    for i in 0..256 {
        let c = if pal.used[i] { pal.colors[i] } else { [0, 0, 0, 255] };
        gp.extend_from_slice(&c[..3]);
    }
    let trans = p.transparent_index();
    let mut out = Vec::new();
    {
        let mut enc = gifc::Encoder::new(&mut out, cw as u16, ch as u16, &gp).map_err(|e| anyhow!("GIF : {e}"))?;
        if p.frames.len() > 1 {
            enc.set_repeat(gifc::Repeat::Infinite).map_err(|e| anyhow!("GIF : {e}"))?;
        }
        for f in &p.frames {
            let OutData::Indexed(idx) = &f.data else { bail!("GIF : image indexée requise") };
            let fr = gifc::Frame {
                width: f.width as u16,
                height: f.height as u16,
                left: (f.pos.0 - x0) as u16,
                top: (f.pos.1 - y0) as u16,
                buffer: std::borrow::Cow::Borrowed(idx),
                transparent: trans,
                delay: (f.delay_ms.unwrap_or(100) / 10).max(1).min(65535) as u16,
                dispose: if p.frames.len() > 1 { gifc::DisposalMethod::Background } else { gifc::DisposalMethod::Keep },
                ..gifc::Frame::default()
            };
            enc.write_frame(&fr).map_err(|e| anyhow!("GIF : {e}"))?;
        }
    }
    Ok(out)
}
