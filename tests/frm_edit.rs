//! Édition de l'en-tête d'un FRM (vitesse, image d'action, décalages) sans réencoder les images.

use fallout_image_browser::formats::{self, DecodeCtx, frm};
use fallout_image_browser::model::*;
use fallout_image_browser::palette;

fn sample() -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join("fallout").join("HMJMPSAA.FRM")).unwrap()
}

fn decode(data: &[u8]) -> Picture {
    let ctx = DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() };
    formats::decode(Format::Frm, data, &ctx).unwrap()
}

#[test]
fn header_is_read_like_the_decoder_sees_it() {
    let data = sample();
    let h = frm::read_header(&data).unwrap();
    let pic = decode(&data);
    assert_eq!(h.directions, pic.directions);
    assert_eq!(h.frames_per_direction as usize, pic.frames_per_dir);
    assert_eq!(h.fps as u32, pic.fps);
    assert_eq!(h.action_frame as u32, pic.action_frame);
    for d in 0..h.directions {
        assert_eq!((h.shift_x[d] as i32, h.shift_y[d] as i32), pic.dir_shift[d]);
    }
}

#[test]
fn patching_changes_only_the_header_fields() {
    let data = sample();
    let h = frm::read_header(&data).unwrap();
    // Mêmes valeurs : le fichier ne change pas d'un octet.
    assert_eq!(frm::patch_header(&data, h.fps, h.action_frame, h.shift_x, h.shift_y).unwrap(), data);

    let (fps, action) = (h.fps.wrapping_add(7), 3u16.min(h.frames_per_direction - 1));
    let (sx, sy) = ([1i16, -2, 3, -4, 5, -6], [10i16, 20, -30, 40, -50, 60]);
    let patched = frm::patch_header(&data, fps, action, sx, sy).unwrap();
    assert_eq!(patched.len(), data.len());
    // Au-delà de l'en-tête (62 octets), rien ne change.
    assert_eq!(&patched[62..], &data[62..]);
    // Les champs modifiés sont ceux qu'on a demandés ; les autres sont intacts.
    let n = frm::read_header(&patched).unwrap();
    assert_eq!((n.fps, n.action_frame, n.shift_x, n.shift_y), (fps, action, sx, sy));
    assert_eq!((n.version, n.frames_per_direction, n.directions), (h.version, h.frames_per_direction, h.directions));
    // Le décodeur voit les nouvelles valeurs et les mêmes images (les positions suivent les nouveaux décalages).
    let (a, b) = (decode(&data), decode(&patched));
    assert_eq!((b.fps, b.action_frame), (fps as u32, action as u32));
    assert_eq!(b.dir_shift[0], (sx[0] as i32, sy[0] as i32));
    assert_eq!(a.frames.len(), b.frames.len());
    for (x, y) in a.frames.iter().zip(&b.frames) {
        let (Pixels::Indexed(px), Pixels::Indexed(py)) = (&x.pixels, &y.pixels) else { panic!("un FRM est indexé") };
        assert!(px == py && x.width == y.width && x.height == y.height);
    }
}

#[test]
fn invalid_requests_are_refused() {
    let data = sample();
    let h = frm::read_header(&data).unwrap();
    // Image d'action inexistante.
    assert!(frm::patch_header(&data, h.fps, h.frames_per_direction, h.shift_x, h.shift_y).is_err());
    // Fichier trop court pour avoir un en-tête.
    assert!(frm::read_header(&data[..40]).is_err());
    assert!(frm::patch_header(&data[..10], 10, 0, [0; 6], [0; 6]).is_err());
}
