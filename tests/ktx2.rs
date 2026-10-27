//! KTX2 : écriture sans perte (RGBA8 + Zstandard) et BC7, relecture, détection par signature.

use fallout_image_browser::convert::*;
use fallout_image_browser::formats::{self, DecodeCtx, EncodeOpts};
use fallout_image_browser::model::*;
use fallout_image_browser::palette;

fn ctx() -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() }
}

/// Image de test en dégradé lisse, de transparence `alpha` ; dimensions volontairement non multiples de 4.
fn picture(w: usize, h: usize, alpha: u8) -> Picture {
    let mut v = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) * 255 / (w + h)) as u8, alpha]);
        }
    }
    Picture::single(Format::Png, w as u32, h as u32, Pixels::Rgba(v), None)
}

fn encode(pic: &Picture, bc7: bool) -> Vec<u8> {
    let enc = EncodeOpts { ktx2_bc7: bc7, ..EncodeOpts::default() };
    let o = ConvertOptions { format: Format::Ktx2, depth: DepthChoice::Rgba32, dither: false, bg: [0, 0, 0], enc, ops: Default::default(), text_white: false };
    convert_to_bytes(pic, &o, &Selection::default()).unwrap()
}

fn rgba(p: &Picture) -> Vec<u8> {
    frame_to_rgba(&p.frames[0], p.palette.as_ref())
}

#[test]
fn lossless_ktx2_roundtrips_exactly() {
    let pic = picture(37, 21, 200);
    let bytes = encode(&pic, false);
    assert_eq!(formats::sniff(&bytes), Some(Format::Ktx2));
    let back = formats::decode(Format::Ktx2, &bytes, &ctx()).unwrap();
    assert_eq!((back.frames[0].width, back.frames[0].height), (37, 21));
    assert_eq!(rgba(&back), rgba(&pic));
}

#[test]
fn bc7_ktx2_is_close_to_the_source() {
    let pic = picture(37, 21, 255);
    let bytes = encode(&pic, true);
    let back = formats::decode(Format::Ktx2, &bytes, &ctx()).unwrap();
    assert_eq!((back.frames[0].width, back.frames[0].height), (37, 21));
    let (a, b) = (rgba(&pic), rgba(&back));
    let mse = a.iter().zip(&b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum::<f64>() / a.len() as f64;
    let psnr = 10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10();
    assert!(psnr > 32.0, "PSNR {psnr:.1} dB trop faible");
}

#[test]
fn truncated_or_unsupported_ktx2_is_an_error() {
    let bytes = encode(&picture(8, 8, 255), false);
    assert!(formats::decode(Format::Ktx2, &bytes[..60], &ctx()).is_err());
    let mut bad = bytes.clone();
    bad[12..16].copy_from_slice(&1u32.to_le_bytes()); // VK_FORMAT_R4G4_UNORM_PACK8
    assert!(formats::decode(Format::Ktx2, &bad, &ctx()).is_err());
}

#[test]
fn bc7_ktx2_keeps_a_constant_alpha() {
    let back = formats::decode(Format::Ktx2, &encode(&picture(16, 16, 100), true), &ctx()).unwrap();
    assert!(rgba(&back).chunks_exact(4).all(|p| (p[3] as i32 - 100).abs() <= 2));
}
