//! JPEG : 24 bits (YCbCr) et 32 bits (CMYK) ; décodage via `image` / zune-jpeg.

use super::*;
use image::{DynamicImage, ImageDecoder};
use std::io::Cursor;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut dec = image::codecs::jpeg::JpegDecoder::new(Cursor::new(data)).map_err(|e| anyhow!("JPEG : {e}"))?;
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(dec).map_err(|e| anyhow!("JPEG : {e}"))?;
    img.apply_orientation(orientation);
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    check_dims(w as u64, h as u64)?;
    let mut pic = Picture::single(Format::Jpeg, w, h, Pixels::Rgba(rgba.into_raw()), None);
    let info = scan_markers(data);
    pic.bpp = info.components as u32 * info.precision as u32;
    let s = pic.section("sec.format");
    s.add("p.compression", info.kind);
    s.add("p.precision", info.precision);
    s.add("p.components", info.components);
    if !info.sampling.is_empty() {
        s.add("p.subsampling", info.sampling);
    }
    if let Some(t) = info.adobe {
        s.raw("Adobe transform", t);
    }
    if let Some(j) = info.jfif {
        s.raw("JFIF", j);
    }
    if let Some(c) = info.comment {
        s.add("p.comment", c);
    }
    if let Some(e) = exif_section(data) {
        pic.sections.push(e);
    }
    Ok(pic)
}

#[derive(Default)]
struct Info {
    kind: String,
    precision: u8,
    components: u8,
    sampling: String,
    adobe: Option<u8>,
    jfif: Option<String>,
    comment: Option<String>,
}

fn scan_markers(d: &[u8]) -> Info {
    let mut info = Info::default();
    let mut i = 2usize;
    while i + 4 <= d.len() {
        if d[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        if m == 0xFF || m == 0 || (0xD0..=0xD8).contains(&m) || m == 0x01 {
            i += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        if m == 0xDA || m == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        let seg = d.get(i + 4..(i + 2 + len).min(d.len())).unwrap_or(&[]);
        match m {
            0xC0..=0xCF if m != 0xC4 && m != 0xC8 && m != 0xCC && seg.len() >= 6 => {
                info.kind = match m {
                    0xC0 => "Baseline DCT",
                    0xC1 => "Extended sequential DCT",
                    0xC2 => "Progressive DCT",
                    0xC3 => "Lossless",
                    0xC9..=0xCB => "Arithmetic",
                    _ => "DCT",
                }
                .to_string();
                info.precision = seg[0];
                info.components = seg[5];
                let mut s = Vec::new();
                for c in 0..info.components as usize {
                    if let Some(b) = seg.get(6 + c * 3 + 1) {
                        s.push(format!("{}×{}", b >> 4, b & 15));
                    }
                }
                info.sampling = s.join(" / ");
            }
            0xE0 if seg.starts_with(b"JFIF\0") && seg.len() >= 12 => {
                let units = match seg[7] {
                    1 => "dpi",
                    2 => "dpcm",
                    _ => "",
                };
                info.jfif = Some(format!("{}.{:02}  {}×{} {}", seg[5], seg[6], u16::from_be_bytes([seg[8], seg[9]]), u16::from_be_bytes([seg[10], seg[11]]), units));
            }
            0xEE if seg.starts_with(b"Adobe") && seg.len() >= 12 => info.adobe = Some(seg[11]),
            0xFE => info.comment = Some(String::from_utf8_lossy(seg).trim().to_string()),
            _ => {}
        }
        i += 2 + len;
    }
    info
}

pub fn encode(p: &Prepared, opts: &EncodeOpts) -> Result<Vec<u8>> {
    let f = p.first();
    let OutData::Rgba(px) = &f.data else { bail!("JPEG : image RVB requise") };
    if f.width > 65535 || f.height > 65535 {
        bail!("image trop grande pour le format JPEG");
    }
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, opts.jpeg_quality.clamp(1, 100));
    if opts.jpeg_quality >= 90 {
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    }
    let (w, h) = (f.width as u16, f.height as u16);
    if p.depth == Depth::Rgba32 {
        // 32 bits = CMYK.
        let mut cmyk = Vec::with_capacity(px.len());
        for c in px.chunks_exact(4) {
            let (r, g, b) = (c[0] as u32, c[1] as u32, c[2] as u32);
            let k = 255 - r.max(g).max(b);
            let d = 255 - k;
            if d == 0 {
                cmyk.extend_from_slice(&[0, 0, 0, 255]);
            } else {
                let cc = ((255 - r - k) * 255 + d / 2) / d;
                let mm = ((255 - g - k) * 255 + d / 2) / d;
                let yy = ((255 - b - k) * 255 + d / 2) / d;
                cmyk.extend_from_slice(&[cc as u8, mm as u8, yy as u8, k as u8]);
            }
        }
        enc.encode(&cmyk, w, h, jpeg_encoder::ColorType::Cmyk).map_err(|e| anyhow!("JPEG : {e}"))?;
    } else {
        enc.encode(px, w, h, jpeg_encoder::ColorType::Rgba).map_err(|e| anyhow!("JPEG : {e}"))?;
    }
    Ok(out)
}
