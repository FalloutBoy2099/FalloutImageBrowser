//! WebP : lecture (statique et animé) et écriture sans perte, via la crate `image`.

use super::*;
use image::{AnimationDecoder, DynamicImage, ImageDecoder};
use std::io::Cursor;

pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut dec = image::codecs::webp::WebPDecoder::new(Cursor::new(data)).map_err(|e| anyhow!("WebP : {e}"))?;
    // Mémoire de décodage bornée : un en-tête mensonger ne doit pas pouvoir réserver une quantité démesurée.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ANIM_BYTES as u64);
    dec.set_limits(limits).map_err(|e| anyhow!("WebP : {e}"))?;
    let animated = dec.has_animation();
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut pic;
    if animated {
        // Images décodées une à une, avec un budget mémoire (et non toutes d'un coup).
        let mut frames = Vec::new();
        for f in dec.into_frames() {
            let f = f.map_err(|e| anyhow!("WebP : {e}"))?;
            check_anim_budget(frames.len() + 1, f.buffer().as_raw().len())?;
            frames.push(f);
        }
        if frames.is_empty() {
            bail!("WebP animé sans image");
        }
        pic = Picture::new(Format::Webp);
        let mut total = 0u32;
        for f in &frames {
            let (n, d) = f.delay().numer_denom_ms();
            let ms = if d == 0 { 100 } else { n / d }.max(10);
            total += ms;
            let b = f.buffer();
            check_dims(b.width() as u64, b.height() as u64)?;
            pic.frames.push(Frame {
                width: b.width(),
                height: b.height(),
                pixels: Pixels::Rgba(b.as_raw().clone()),
                pos: (0, 0),
                delay_ms: Some(ms),
            });
        }
        pic.frames_per_dir = pic.frames.len();
        pic.fps = (pic.frames.len() as u32 * 1000 / total.max(1)).max(1);
    } else {
        let mut img = DynamicImage::from_decoder(dec).map_err(|e| anyhow!("WebP : {e}"))?;
        img.apply_orientation(orientation);
        let rgba = img.to_rgba8();
        check_dims(rgba.width() as u64, rgba.height() as u64)?;
        pic = Picture::single(Format::Webp, rgba.width(), rgba.height(), Pixels::Rgba(rgba.into_raw()), None);
    }
    let mut kind = String::new();
    let mut flags = 0u8;
    let mut i = 12usize;
    while i + 8 <= data.len() {
        let tag = &data[i..i + 4];
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        match tag {
            b"VP8 " => kind = "VP8 (lossy)".into(),
            b"VP8L" => kind = "VP8L (lossless)".into(),
            b"VP8X" => flags = data.get(i + 8).copied().unwrap_or(0),
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    let nframes = pic.frames.len();
    let alpha = flags & 0x10 != 0 || pic.frames.iter().any(|f| matches!(&f.pixels, Pixels::Rgba(v) if v.chunks_exact(4).any(|p| p[3] < 255)));
    pic.bpp = if alpha { 32 } else { 24 };
    let s = pic.section("sec.format");
    if !kind.is_empty() {
        s.add("p.compression", kind);
    }
    s.add("p.animated", if animated { "p.yes" } else { "p.no" });
    if animated {
        s.add("p.frames", nframes);
    }
    if flags != 0 {
        s.raw("Features", format!(
            "{}{}{}{}{}",
            if flags & 0x20 != 0 { "ICC " } else { "" },
            if flags & 0x10 != 0 { "Alpha " } else { "" },
            if flags & 0x08 != 0 { "EXIF " } else { "" },
            if flags & 0x04 != 0 { "XMP " } else { "" },
            if flags & 0x02 != 0 { "Animation" } else { "" },
        ));
    }
    if let Some(e) = exif_section(data) {
        pic.sections.push(e);
    }
    Ok(pic)
}

pub fn encode(p: &Prepared) -> Result<Vec<u8>> {
    let f = p.first();
    let OutData::Rgba(px) = &f.data else { bail!("WebP : image RVB requise") };
    let mut out = Vec::new();
    let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
    if p.depth == Depth::Rgba32 {
        enc.encode(px, f.width, f.height, image::ExtendedColorType::Rgba8).map_err(|e| anyhow!("WebP : {e}"))?;
    } else {
        let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
        enc.encode(&rgb, f.width, f.height, image::ExtendedColorType::Rgb8).map_err(|e| anyhow!("WebP : {e}"))?;
    }
    Ok(out)
}
