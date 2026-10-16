//! Création d'une animation FRM / ART à partir des images d'un dossier, et empreinte SHA-256.

use fallout_image_browser::batch::build_animation;
use fallout_image_browser::hash::sha256_hex;
use fallout_image_browser::loader::{PalPolicy, load_file};
use fallout_image_browser::model::Format;

fn ppm(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
    let mut v = format!("P6\n{w} {h}\n255\n").into_bytes();
    for _ in 0..w * h {
        v.extend_from_slice(&rgb);
    }
    v
}

#[test]
fn folder_images_become_an_animation() {
    let dir = std::env::temp_dir().join(format!("fib_anim_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // L'ordre est celui des noms (naturel) : img2 avant img10.
    std::fs::write(dir.join("img10.ppm"), ppm(5, 5, [0, 0, 255])).unwrap();
    std::fs::write(dir.join("img2.ppm"), ppm(6, 4, [255, 0, 0])).unwrap();
    std::fs::write(dir.join("img1.ppm"), ppm(4, 4, [0, 255, 0])).unwrap();
    std::fs::write(dir.join("readme.txt"), b"ignored").unwrap();
    for fmt in [Format::Frm, Format::Art] {
        let (out, n) = build_animation(&dir, &dir, "walk", fmt, &PalPolicy::Fallout).unwrap();
        assert_eq!(n, 3);
        assert_eq!(out.file_name().unwrap().to_string_lossy().to_lowercase(), format!("walk.{}", fmt.ext()));
        assert_eq!(out.extension().unwrap(), fmt.ext());
        let l = load_file(&out, &PalPolicy::Fallout).unwrap();
        assert_eq!(l.pic.frames.len(), 3);
        assert_eq!((l.pic.frames[0].width, l.pic.frames[1].width, l.pic.frames[2].width), (4, 6, 5));
        // Un second appel ne remplace pas le premier fichier.
        assert!(build_animation(&dir, &dir, "walk", fmt, &PalPolicy::Fallout).is_err());
    }
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(build_animation(&empty, &empty, "x", Format::Frm, &PalPolicy::Fallout).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sha256_is_lowercase_hex() {
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}
