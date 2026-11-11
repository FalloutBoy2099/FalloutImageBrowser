//! Assemblage PNG -> FRM, remappage de palette, vignette en ligne de commande, animation de palette, FR0-FR5.

use fallout_image_browser::cli;
use fallout_image_browser::formats::frm;
use fallout_image_browser::loader::{PalPolicy, load_file};
use fallout_image_browser::model::*;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fib_newfeat_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_png(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
    let mut img = image::RgbaImage::new(w, h);
    for p in img.pixels_mut() {
        *p = image::Rgba([rgb[0], rgb[1], rgb[2], 255]);
    }
    img.save(path).unwrap();
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn assemble_command_builds_a_two_direction_frm() {
    let d = tmp("asm");
    let mut inputs = Vec::new();
    for (i, c) in [[252, 0, 0], [0, 252, 0], [0, 0, 252], [252, 252, 0]].iter().enumerate() {
        let p = d.join(format!("f{i}.png"));
        write_png(&p, 8 + i as u32, 6, *c);
        inputs.push(p.to_string_lossy().to_string());
    }
    let out = d.join("anim.frm");
    let mut a = vec!["assemble".to_string()];
    a.extend(inputs);
    a.extend(args(&["-o", out.to_str().unwrap(), "--directions", "2", "--fps", "12", "--action", "1"]));
    assert_eq!(cli::run(&a), 0);
    let data = std::fs::read(&out).unwrap();
    let h = frm::read_header(&data).unwrap();
    assert_eq!((h.directions, h.frames_per_direction, h.fps, h.action_frame), (2, 2, 12, 1));
    let loaded = load_file(&out, &PalPolicy::Fallout).unwrap();
    assert_eq!(loaded.pic.frames.len(), 4);
    assert_eq!(loaded.pic.frames[1].width, 9);
    // Une seconde exécution ne remplace jamais le fichier existant.
    assert_eq!(cli::run(&a), 0);
    assert!(d.join("anim_converted.frm").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn assemble_refuses_uneven_split() {
    let d = tmp("uneven");
    let mut a = vec!["assemble".to_string()];
    for i in 0..3 {
        let p = d.join(format!("f{i}.png"));
        write_png(&p, 4, 4, [252, 0, 0]);
        a.push(p.to_string_lossy().to_string());
    }
    a.extend(args(&["-o", d.join("x.frm").to_str().unwrap(), "--directions", "2"]));
    assert_eq!(cli::run(&a), 1);
    assert!(!d.join("x.frm").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn remap_to_the_same_palette_keeps_the_pixels() {
    let d = tmp("remap");
    let src = d.join("EXP.FRM");
    std::fs::copy("_test_files/fallout/EXP.FRM", &src).unwrap();
    let out = d.join("out.frm");
    assert_eq!(cli::run(&args(&["remap", src.to_str().unwrap(), "-p", "fallout", "-o", out.to_str().unwrap()])), 0);
    let a = load_file(&src, &PalPolicy::Fallout).unwrap();
    let b = load_file(&out, &PalPolicy::Fallout).unwrap();
    assert_eq!(a.pic.frames.len(), b.pic.frames.len());
    for (x, y) in a.pic.frames.iter().zip(&b.pic.frames) {
        assert_eq!(frame_to_rgba(x, a.pic.palette.as_ref()), frame_to_rgba(y, b.pic.palette.as_ref()));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn remap_to_a_reduced_palette_changes_indexes() {
    let d = tmp("remap2");
    let src = d.join("HEX.frm");
    std::fs::copy("_test_files/fallout/HEX.frm", &src).unwrap();
    // Palette cible : niveaux de gris seulement.
    let mut pal = Palette::empty();
    for i in 0..256usize {
        let v = (i as u8) & 0xFC;
        pal.colors[i] = [v, v, v, 255];
        pal.used[i] = true;
    }
    pal.colors[0][3] = 0;
    let pal_file = d.join("target_grey.pal");
    std::fs::write(&pal_file, fallout_image_browser::palette::write_pal(&pal)).unwrap();
    let out = d.join("grey.frm");
    assert_eq!(cli::run(&args(&["remap", src.to_str().unwrap(), "-p", pal_file.to_str().unwrap(), "-o", out.to_str().unwrap()])), 0);
    // Le FRM n'embarque pas sa palette : la palette cible est écrite à côté.
    let shipped = std::fs::read(out.with_extension("pal")).expect("grey.pal");
    let shipped = fallout_image_browser::palette::parse_pal(&shipped).unwrap();
    assert_eq!(shipped.colors[100][..3], pal.colors[100][..3]);
    let b = load_file(&out, &PalPolicy::Custom(std::sync::Arc::new(pal), "grey".into())).unwrap();
    let rgba = frame_to_rgba(&b.pic.frames[0], b.pic.palette.as_ref());
    assert!(rgba.chunks_exact(4).all(|p| p[3] == 0 || (p[0] == p[1] && p[1] == p[2])));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn thumbnail_command_writes_a_small_png() {
    let d = tmp("thumb");
    let out = d.join("t.png");
    assert_eq!(cli::run(&args(&["thumbnail", "_test_files/fallout/SPLASH0.rix", out.to_str().unwrap(), "--size", "64"])), 0);
    let img = image::open(&out).unwrap();
    assert!(img.width().max(img.height()) <= 64);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn per_direction_extensions_are_read_as_frm() {
    for n in 0..6 {
        assert_eq!(Format::from_ext(&format!("fr{n}")), Some(Format::Frm));
        assert_eq!(Format::from_ext(&format!("FR{n}")), Some(Format::Frm));
    }
    // Un FRM d'une seule direction enregistré sous .FR3 se relit.
    let d = tmp("fr3");
    let src = d.join("one.png");
    write_png(&src, 5, 5, [252, 0, 0]);
    let out = d.join("one.frm");
    assert_eq!(cli::run(&args(&["assemble", src.to_str().unwrap(), "-o", out.to_str().unwrap()])), 0);
    let fr3 = d.join("one.FR3");
    std::fs::rename(&out, &fr3).unwrap();
    let l = load_file(&fr3, &PalPolicy::Fallout).unwrap();
    assert_eq!((l.pic.format, l.pic.directions, l.pic.frames.len()), (Format::Frm, 1, 1));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_linux_thumbnailer_declares_every_extension() {
    let t = fallout_image_browser::assoc::thumbnailer_entry(Path::new("/opt/fib/fallout_image_browser"));
    assert!(t.starts_with("[Thumbnailer Entry]\n"));
    assert!(t.contains("Exec=\"/opt/fib/fallout_image_browser\" thumbnail %i %o --size %s"));
    for ty in fallout_image_browser::assoc::all() {
        assert!(t.contains(&fallout_image_browser::assoc::mime_type(ty.ext)), "{}", ty.ext);
    }
}

#[test]
fn fallout_palette_has_animated_colours_that_rotate() {
    let pal = fallout_image_browser::palette::fallout();
    assert!(fallout_image_browser::cycling::has_cycling(&pal));
    let a = fallout_image_browser::cycling::cycled(&pal, 0);
    let b = fallout_image_browser::cycling::cycled(&pal, 150);
    assert_ne!(a.colors[229..=253], b.colors[229..=253]);
    assert_eq!(a.colors[..229], b.colors[..229]);
}

#[test]
fn frm_in_an_archive_uses_the_color_pal_of_that_archive() {
    use fallout_image_browser::dat;
    let d = tmp("colorpal");
    let frm = std::fs::read("_test_files/fallout/EXP.FRM").unwrap();
    // color.pal du jeu simulée : Fallout, mais avec l'index 229 en vert vif (6 bits).
    let mut raw = std::fs::read("assets/default.pal").unwrap();
    for i in 229..256 {
        raw[i * 3..i * 3 + 3].copy_from_slice(&[0, 40, 0]);
    }
    let with_pal = dat::write_dat2(&[(r"art\misc\exp.frm", &frm, true), ("color.pal", &raw, true)]);
    let without = dat::write_dat2(&[(r"art\misc\exp.frm", &frm, true)]);
    std::fs::write(d.join("a.dat"), with_pal).unwrap();
    std::fs::write(d.join("b.dat"), without).unwrap();
    let a = load_file(&d.join("a.dat").join("art").join("misc").join("exp.frm"), &PalPolicy::Auto).unwrap();
    assert!(a.pic.palette.as_ref().unwrap().used[229]);
    assert_eq!(a.pic.palette.as_ref().unwrap().colors[229], [0, 160, 0, 255]);
    assert!(a.pic.sections.iter().flat_map(|s| s.items.iter()).any(|p| format!("{p:?}").contains("color.pal")));
    let b = load_file(&d.join("b.dat").join("art").join("misc").join("exp.frm"), &PalPolicy::Auto).unwrap();
    assert!(!b.pic.palette.as_ref().unwrap().used[229]);
    // Un choix explicite de palette garde la priorité.
    let c = load_file(&d.join("a.dat").join("art").join("misc").join("exp.frm"), &PalPolicy::Fallout).unwrap();
    assert!(!c.pic.palette.as_ref().unwrap().used[229]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn reassembling_a_frm_keeps_offsets_and_direction_shifts() {
    let d = tmp("reasm");
    let src = d.join("HMJMPSAA.FRM");
    std::fs::copy("_test_files/fallout/HMJMPSAA.FRM", &src).unwrap();
    let a = load_file(&src, &PalPolicy::Fallout).unwrap();
    let out = d.join("again.frm");
    let dirs = a.pic.directions.to_string();
    assert_eq!(cli::run(&args(&["assemble", src.to_str().unwrap(), "-o", out.to_str().unwrap(), "--directions", &dirs, "--fps", &a.pic.fps.to_string()])), 0);
    let b = load_file(&out, &PalPolicy::Fallout).unwrap();
    assert_eq!(a.pic.dir_shift, b.pic.dir_shift);
    assert_eq!(a.pic.frames.len(), b.pic.frames.len());
    for (x, y) in a.pic.frames.iter().zip(&b.pic.frames) {
        assert_eq!((x.width, x.height, x.pos), (y.width, y.height, y.pos));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn textures_are_shrunk_to_the_gpu_limit() {
    use fallout_image_browser::view::fit_texture;
    let (w, h, v) = fit_texture(vec![255; 100 * 40 * 4], 100, 40, 50);
    assert_eq!((w, h, v.len()), (50, 20, 50 * 20 * 4));
    let (w, h, v) = fit_texture(vec![1; 10 * 10 * 4], 10, 10, 50);
    assert_eq!((w, h, v.len()), (10, 10, 400));
}

#[test]
fn grid_alignment_of_palettes() {
    let fallout = fallout_image_browser::palette::fallout();
    assert!(fallout_image_browser::palette::is_grid_aligned(&fallout));
    let mut p = (*fallout).clone();
    p.colors[10] = [253, 1, 2, 255];
    assert!(!fallout_image_browser::palette::is_grid_aligned(&p));
}
