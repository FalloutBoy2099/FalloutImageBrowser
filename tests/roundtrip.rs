use fallout_image_browser::convert::*;
use fallout_image_browser::formats::{self, DecodeCtx, EncodeOpts};
use fallout_image_browser::model::*;
use fallout_image_browser::palette;
use std::path::Path;

fn ctx() -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() }
}

fn synthetic(w: usize, h: usize) -> Picture {
    let mut v = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            v[o] = (x * 255 / (w - 1)) as u8;
            v[o + 1] = (y * 255 / (h - 1)) as u8;
            v[o + 2] = ((x + y) * 255 / (w + h - 2)) as u8;
            let dx = x as i32 - w as i32 / 2;
            let dy = y as i32 - h as i32 / 2;
            v[o + 3] = if dx * dx + dy * dy < 100 { 0 } else { 255 };
        }
    }
    Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(v), None)
}

fn modes() -> Vec<(Format, DepthChoice, String)> {
    let mut m = Vec::new();
    for f in Format::ALL {
        let c = formats::caps(f);
        if c.indexed {
            if !c.no_adaptive {
                m.push((f, DepthChoice::Indexed(PalChoice::Adaptive), format!("{} 8-bit", f.name())));
            }
            m.push((f, DepthChoice::Indexed(PalChoice::Fallout), format!("{} 8-bit Fallout", f.name())));
            if c.custom_pal {
                m.push((f, DepthChoice::Indexed(PalChoice::Custom(palette::fallout())), format!("{} 8-bit PAL", f.name())));
            }
        }
        if c.d16 {
            m.push((f, DepthChoice::Rgb16, format!("{} 16-bit", f.name())));
        }
        if c.d24 {
            m.push((f, DepthChoice::Rgb24, format!("{} 24-bit", f.name())));
        }
        if c.d32 {
            m.push((f, DepthChoice::Rgba32, format!("{} 32-bit", f.name())));
        }
    }
    m
}

fn opts(f: Format, d: DepthChoice) -> ConvertOptions {
    ConvertOptions { format: f, depth: d, dither: false, bg: [0, 0, 0], enc: EncodeOpts::default(), ops: Default::default() }
}

#[test]
fn synthetic_roundtrip_all_modes() {
    let src = synthetic(64, 48);
    let Pixels::Rgba(orig) = &src.frames[0].pixels else { unreachable!() };
    let mut fails = Vec::new();
    for (f, d, name) in modes() {
        let o = opts(f, d.clone());
        let bytes = match convert_to_bytes(&src, &o, &Selection::default()) {
            Ok(b) => b,
            Err(e) => {
                fails.push(format!("{name}: encode {e}"));
                continue;
            }
        };
        let pic = match formats::decode(f, &bytes, &ctx()) {
            Ok(p) => p,
            Err(e) => {
                fails.push(format!("{name}: decode {e}"));
                continue;
            }
        };
        let fr = &pic.frames[0];
        if (fr.width, fr.height) != (64, 48) {
            fails.push(format!("{name}: dims {}x{}", fr.width, fr.height));
            continue;
        }
        let rgba = frame_to_rgba(fr, pic.palette.as_ref());
        // Écart moyen par rapport à l'original (sur les pixels opaques).
        let (mut sum, mut n) = (0u64, 0u64);
        let mut alpha_ok = true;
        for i in 0..64 * 48 {
            if orig[i * 4 + 3] == 255 {
                for c in 0..3 {
                    sum += (orig[i * 4 + c] as i32 - rgba[i * 4 + c] as i32).unsigned_abs() as u64;
                }
                n += 3;
            }
            let keeps = rgba[i * 4 + 3] < 255;
            if (orig[i * 4 + 3] == 0) != keeps {
                alpha_ok = false;
            }
        }
        let avg = sum as f64 / n as f64;
        let c = formats::caps(f);
        let expects_alpha = match &d {
            DepthChoice::Indexed(_) => c.indexed_alpha,
            DepthChoice::Rgba32 => f != Format::Jpeg,
            DepthChoice::Rgb16 => matches!(f, Format::Tga | Format::Dds),
            _ => false,
        };
        println!("{name:<22} {:>7} B  avg err {avg:5.2}  alpha {}", bytes.len(), if alpha_ok { "ok" } else { "differs" });
        let tol = match &d {
            DepthChoice::Rgba32 | DepthChoice::Rgb24 if f != Format::Jpeg => 0.001,
            DepthChoice::Rgb16 => 4.0,
            DepthChoice::Indexed(PalChoice::Adaptive) => 8.0,
            DepthChoice::Indexed(_) => 12.0,
            _ => 30.0,
        };
        if avg > tol * (if matches!(d, DepthChoice::Indexed(PalChoice::Fallout) | DepthChoice::Indexed(PalChoice::Custom(_))) { 4.0 } else { 1.0 }) {
            fails.push(format!("{name}: erreur moyenne {avg:.2} > {tol}"));
        }
        if expects_alpha && !alpha_ok {
            fails.push(format!("{name}: transparence perdue"));
        }
    }
    assert!(fails.is_empty(), "échecs :\n{}", fails.join("\n"));
}

/// Fichiers d'exemple versionnés avec le projet (dossier `_test_files`).
fn sample(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join(rel)
}

/// Charge un fichier d'exemple (chemin relatif à `_test_files`).
fn load(rel: &str) -> Option<(Picture, Vec<u8>)> {
    let p = sample(rel);
    let data = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let f = Format::from_path(&p).unwrap();
    Some((formats::decode(f, &data, &ctx()).unwrap_or_else(|e| panic!("{rel}: {e}")), data))
}

#[test]
fn real_samples_convert_everywhere() {
    let samples = ["fallout/HEX.frm", "fallout/SPLASH0.rix", "arcanum/MusicBoxMap.art", "arcanum/TarantSewer.art"];
    let mut fails = Vec::new();
    for s in samples {
        let Some((pic, _)) = load(s) else { continue };
        println!(
            "{s}: {}x{} dirs {} fpd {} frames {} colors {}",
            pic.frames[0].width,
            pic.frames[0].height,
            pic.directions,
            pic.frames_per_dir,
            pic.frames.len(),
            pic.unique_colors
        );
        for (f, d, name) in modes() {
            let o = opts(f, d);
            match convert_to_bytes(&pic, &o, &Selection::default()) {
                Ok(b) => {
                    if let Err(e) = formats::decode(f, &b, &ctx()) {
                        fails.push(format!("{s} -> {name}: relecture {e}"));
                    }
                }
                Err(e) => fails.push(format!("{s} -> {name}: {e}")),
            }
        }
    }
    assert!(fails.is_empty(), "échecs :\n{}", fails.join("\n"));
}

#[test]
fn frm_identity() {
    let dir = sample("fallout");
    let mut n = 0;
    let mut bad = Vec::new();
    for e in walk(&dir) {
        if e.extension().map(|x| x.eq_ignore_ascii_case("frm")).unwrap_or(false) {
            let data = std::fs::read(&e).unwrap();
            let Ok(pic) = formats::decode(Format::Frm, &data, &ctx()) else { continue };
            let o = opts(Format::Frm, DepthChoice::Indexed(PalChoice::Fallout));
            let out = convert_to_bytes(&pic, &o, &Selection::default()).unwrap();
            let again = formats::decode(Format::Frm, &out, &ctx()).unwrap();
            let same = again.frames.len() == pic.frames.len()
                && again.frames.iter().zip(&pic.frames).all(|(a, b)| {
                    a.width == b.width && a.height == b.height && a.pos == b.pos && matches!((&a.pixels, &b.pixels), (Pixels::Indexed(x), Pixels::Indexed(y)) if x == y)
                });
            if !same {
                bad.push(e.display().to_string());
            }
            if out != data && same {
                // Les octets diffèrent (version, fps...) mais pas le contenu : toléré.
            }
            n += 1;
            if n >= 3000 {
                break;
            }
        }
    }
    println!("{n} FRM testés");
    assert!(n >= 3, "FRM d'exemple introuvables dans _test_files/fallout");
    assert!(bad.is_empty(), "FRM non identiques : {bad:?}");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                v.extend(walk(&p));
            } else {
                v.push(p);
            }
        }
    }
    v
}

/// Écrit quelques rendus en JPEG dans un dossier pour inspection visuelle.
#[test]
fn dump_previews() {
    let Ok(out) = std::env::var("FIB_DUMP") else { return };
    let samples = [("fallout/HEX.frm", "frm"), ("fallout/SPLASH0.rix", "rix"), ("arcanum/MusicBoxMap.art", "art1"), ("arcanum/TarantSewer.art", "art2")];
    for (s, name) in samples {
        let Some((pic, _)) = load(s) else { continue };
        let o = opts(Format::Jpeg, DepthChoice::Rgb24);
        let b = convert_to_bytes(&pic, &o, &Selection::default()).unwrap();
        std::fs::write(Path::new(&out).join(format!("{name}.jpg")), b).unwrap();
    }
}

/// Génère une grande image de test (FIB_BIG=chemin.jpg) pour mesurer la fluidité de l'affichage.
#[test]
fn make_big_image() {
    let Ok(out) = std::env::var("FIB_BIG") else { return };
    let (w, h) = (8000usize, 5000usize);
    let mut v = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            let n = ((x * 7919 + y * 104729) ^ (x * y)) as u8;
            v[o] = ((x * 255 / w) as u8).wrapping_add(n / 8);
            v[o + 1] = ((y * 255 / h) as u8).wrapping_add(n / 8);
            v[o + 2] = (((x / 40 + y / 40) % 2) * 120) as u8 + n / 8;
        }
    }
    let pic = Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(v), None);
    let b = convert_to_bytes(&pic, &opts(Format::Jpeg, DepthChoice::Rgb24), &Selection::default()).unwrap();
    std::fs::write(out, b).unwrap();
}

#[test]
fn critter_animation_conversions() {
    let Some((pic, _)) = load("fallout/HMJMPSAA.FRM") else { return };
    assert_eq!((pic.directions, pic.frames_per_dir), (6, 12));
    // FRM -> GIF animé (direction 2)
    let o = opts(Format::Gif, DepthChoice::Indexed(PalChoice::Fallout));
    let b = convert_to_bytes(&pic, &o, &Selection { dir: 2, frame: 0 }).unwrap();
    let g = formats::decode(Format::Gif, &b, &ctx()).unwrap();
    assert_eq!(g.frames.len(), 12);
    // FRM -> ART : 8 rotations
    let o = opts(Format::Art, DepthChoice::Indexed(PalChoice::Fallout));
    let b = convert_to_bytes(&pic, &o, &Selection::default()).unwrap();
    let a = formats::decode(Format::Art, &b, &ctx()).unwrap();
    assert_eq!((a.directions, a.frames_per_dir), (8, 12));
    // ART -> FRM : 6 directions conservées
    let o = opts(Format::Frm, DepthChoice::Indexed(PalChoice::Fallout));
    let b = convert_to_bytes(&a, &o, &Selection::default()).unwrap();
    let f = formats::decode(Format::Frm, &b, &ctx()).unwrap();
    assert_eq!((f.directions, f.frames_per_dir), (6, 12));
    // GIF animé -> FRM (une direction)
    let b = convert_to_bytes(&g, &o, &Selection::default()).unwrap();
    let f2 = formats::decode(Format::Frm, &b, &ctx()).unwrap();
    assert_eq!((f2.directions, f2.frames_per_dir), (1, 12));
    // FRM -> BMP 8 bits Fallout : indices conservés pour l'image courante
    let o = opts(Format::Bmp, DepthChoice::Indexed(PalChoice::Fallout));
    let b = convert_to_bytes(&pic, &o, &Selection { dir: 1, frame: 3 }).unwrap();
    let bm = formats::decode(Format::Bmp, &b, &ctx()).unwrap();
    let (Pixels::Indexed(x), Pixels::Indexed(y)) = (&bm.frames[0].pixels, &pic.frames[pic.frame_index(1, 3)].pixels) else { panic!() };
    assert_eq!(x.len(), y.len());
    // l'index 0 (transparent) reste 0, le reste identique
    assert!(x.iter().zip(y).all(|(a, b)| a == b));
}
