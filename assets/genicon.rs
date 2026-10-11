//! Régénère les icônes à partir de l'illustration `assets/icon_source.webp` (ou d'une autre image donnée en argument) :
//! `assets/icon.ico` (16 à 256 px, pour l'exécutable), `assets/icon128.png` (icône de fenêtre) et `assets/icon.png` (512 px).
//! `cargo run --release --example genicon [image-source]`

use image::imageops::{self, FilterType};

/// Marge rognée sur chaque bord de la source (liseré clair de l'illustration).
const TRIM: u32 = 5;

fn main() {
    let src = std::env::args().nth(1).unwrap_or_else(|| "assets/icon_source.webp".to_string());
    let img = image::open(&src).unwrap_or_else(|e| panic!("{src} : {e}")).to_rgba8();
    // Recadrage carré centré.
    let side = img.width().min(img.height()) - 2 * TRIM;
    let (x0, y0) = ((img.width() - side) / 2, (img.height() - side) / 2);
    let square = imageops::crop_imm(&img, x0, y0, side, side).to_image();
    let at = |s: u32| imageops::resize(&square, s, s, FilterType::Lanczos3);

    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let images: Vec<(u32, Vec<u8>)> = sizes.iter().map(|&s| (s, at(s).into_raw())).collect();
    std::fs::write("assets/icon.ico", fallout_image_browser::icon::to_ico(&images)).expect("écriture de assets/icon.ico");
    at(128).save("assets/icon128.png").expect("écriture de assets/icon128.png");
    at(512).save("assets/icon.png").expect("écriture de assets/icon.png");
    println!("assets/icon.ico, icon128.png et icon.png écrits (source {}x{}, recadrée en {side}x{side})", img.width(), img.height());
}
