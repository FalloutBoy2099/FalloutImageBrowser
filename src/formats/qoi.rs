//! QOI (« Quite OK Image ») : 24 bits (RVB) et 32 bits (RVBA), https://qoiformat.org/qoi-specification.pdf

use super::*;

const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xC0;
const OP_RGB: u8 = 0xFE;
const OP_RGBA: u8 = 0xFF;

fn hash(p: [u8; 4]) -> usize {
    (p[0] as usize * 3 + p[1] as usize * 5 + p[2] as usize * 7 + p[3] as usize * 11) % 64
}

pub fn decode(data: &[u8]) -> Result<Picture> {
    if data.len() < 22 || &data[..4] != b"qoif" {
        bail!("signature QOI absente");
    }
    let w = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
    let h = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;
    let (channels, colorspace) = (data[12], data[13]);
    check_dims(w as u64, h as u64)?;
    if channels != 3 && channels != 4 {
        bail!("QOI : nombre de canaux invalide ({channels})");
    }
    let body = &data[14..data.len() - 8];
    let mut out = vec![255u8; w * h * 4];
    let mut index = [[0u8; 4]; 64];
    let mut px = [0u8, 0, 0, 255];
    let (mut p, mut run) = (0usize, 0u32);
    for i in 0..w * h {
        if run > 0 {
            run -= 1;
        } else if p < body.len() {
            let b1 = body[p];
            p += 1;
            let need = |n: usize| -> Result<()> { if p + n > body.len() { bail!("QOI : données tronquées") } else { Ok(()) } };
            match b1 {
                OP_RGB => {
                    need(3)?;
                    px[..3].copy_from_slice(&body[p..p + 3]);
                    p += 3;
                }
                OP_RGBA => {
                    need(4)?;
                    px.copy_from_slice(&body[p..p + 4]);
                    p += 4;
                }
                _ => match b1 & 0xC0 {
                    OP_INDEX => px = index[b1 as usize],
                    OP_DIFF => {
                        px[0] = px[0].wrapping_add(((b1 >> 4) & 3).wrapping_sub(2));
                        px[1] = px[1].wrapping_add(((b1 >> 2) & 3).wrapping_sub(2));
                        px[2] = px[2].wrapping_add((b1 & 3).wrapping_sub(2));
                    }
                    OP_LUMA => {
                        need(1)?;
                        let b2 = body[p];
                        p += 1;
                        let vg = (b1 & 0x3F).wrapping_sub(32);
                        px[0] = px[0].wrapping_add(vg.wrapping_sub(8).wrapping_add((b2 >> 4) & 0x0F));
                        px[1] = px[1].wrapping_add(vg);
                        px[2] = px[2].wrapping_add(vg.wrapping_sub(8).wrapping_add(b2 & 0x0F));
                    }
                    _ => run = (b1 & 0x3F) as u32, // OP_RUN
                },
            }
            index[hash(px)] = px;
        }
        out[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    let mut pic = Picture::single(Format::Qoi, w as u32, h as u32, Pixels::Rgba(out), None);
    pic.bpp = channels as u32 * 8;
    let s = pic.section("sec.format");
    s.add("p.compression", "QOI");
    s.raw("Channels", channels);
    s.raw("Colorspace", if colorspace == 0 { "sRGB, linear alpha" } else { "linear" });
    Ok(pic)
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let OutData::Rgba(px) = &f.data else { bail!("QOI : image RVB requise") };
    let channels = if p.depth == Depth::Rgba32 { 4u8 } else { 3 };
    let mut o: Vec<u8> = Vec::with_capacity(px.len() / 2 + 32);
    o.extend_from_slice(b"qoif");
    o.extend_from_slice(&f.width.to_be_bytes());
    o.extend_from_slice(&f.height.to_be_bytes());
    o.push(channels);
    o.push(0);
    let mut index = [[0u8; 4]; 64];
    let mut prev = [0u8, 0, 0, 255];
    let mut run = 0u8;
    let n = px.len() / 4;
    for (i, c) in px.chunks_exact(4).enumerate() {
        let cur = [c[0], c[1], c[2], if channels == 4 { c[3] } else { 255 }];
        if cur == prev {
            run += 1;
            if run == 62 || i == n - 1 {
                o.push(OP_RUN | (run - 1));
                run = 0;
            }
        } else {
            if run > 0 {
                o.push(OP_RUN | (run - 1));
                run = 0;
            }
            let ix = hash(cur);
            if index[ix] == cur {
                o.push(OP_INDEX | ix as u8);
            } else {
                index[ix] = cur;
                if cur[3] == prev[3] {
                    let (dr, dg, db) = (cur[0].wrapping_sub(prev[0]) as i8, cur[1].wrapping_sub(prev[1]) as i8, cur[2].wrapping_sub(prev[2]) as i8);
                    let (drg, dbg) = (dr.wrapping_sub(dg), db.wrapping_sub(dg));
                    if (-2..=1).contains(&dr) && (-2..=1).contains(&dg) && (-2..=1).contains(&db) {
                        o.push(OP_DIFF | ((dr + 2) as u8) << 4 | ((dg + 2) as u8) << 2 | (db + 2) as u8);
                    } else if (-32..=31).contains(&dg) && (-8..=7).contains(&drg) && (-8..=7).contains(&dbg) {
                        o.push(OP_LUMA | (dg + 32) as u8);
                        o.push(((drg + 8) as u8) << 4 | (dbg + 8) as u8);
                    } else {
                        o.extend_from_slice(&[OP_RGB, cur[0], cur[1], cur[2]]);
                    }
                } else {
                    o.extend_from_slice(&[OP_RGBA, cur[0], cur[1], cur[2], cur[3]]);
                }
            }
        }
        prev = cur;
    }
    o.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    Ok(o)
}
