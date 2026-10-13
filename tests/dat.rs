use fallout_image_browser::cli;
use fallout_image_browser::dat::{self, Stat};
use fallout_image_browser::model::*;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fib_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Fichiers d'exemple versionnés avec le projet (dossier `_test_files`).
fn sample(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join(rel)
}

fn sample_frm() -> Vec<u8> {
    std::fs::read(sample("fallout/EXP.FRM")).expect("_test_files/fallout/EXP.FRM")
}

#[test]
fn synthetic_dat2_and_dat1_roundtrip() {
    let dir = tmp("dat");
    let frm = sample_frm();
    let big: Vec<u8> = (0..40000u32).map(|i| (i * 7 % 251) as u8).collect();
    let d2 = dat::write_dat2(&[(r"art\misc\a.frm", &frm, true), (r"art\misc\b.frm", &frm, false), ("readme.txt", b"hello", true)]);
    std::fs::write(dir.join("f2.dat"), d2).unwrap();
    let d1 = dat::write_dat1(&[(".", vec![("root.bin", big.clone(), true)]), (r"ART\CRITTERS", vec![("hmwarr.frm", frm.clone(), true), ("plain.frm", frm.clone(), false)])]);
    std::fs::write(dir.join("f1.dat"), d1).unwrap();
    for (name, kind) in [("f2.dat", dat::DatKind::Dat2), ("f1.dat", dat::DatKind::Dat1)] {
        let p = dir.join(name);
        let d = dat::Dat::open(&p).unwrap();
        assert_eq!(d.kind, kind);
        for i in 0..d.entries.len() {
            let b = d.read(i).unwrap();
            let e = &d.entries[i];
            let want: &[u8] = if e.path.ends_with("frm") {
                &frm
            } else if e.path.ends_with("root.bin") {
                &big
            } else {
                b"hello"
            };
            assert_eq!(b, want, "{name}: {}", e.path);
        }
    }
    // Chemins virtuels, insensibles à la casse.
    let v = dir.join("f1.dat").join("art").join("critters").join("HMWARR.FRM");
    assert_eq!(dat::stat(&v), Some(Stat::File));
    assert_eq!(dat::stat(&dir.join("f1.dat").join("art")), Some(Stat::Dir));
    assert_eq!(dat::read_any(&v).unwrap(), frm);
    let (dirs, _) = dat::Dat::open(&dir.join("f1.dat")).unwrap().children("");
    assert_eq!(dirs, vec!["ART".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_file_dir_and_archive_modes() {
    let dir = tmp("cli");
    let frm = sample_frm();
    std::fs::write(dir.join("one.frm"), &frm).unwrap();
    std::fs::write(dir.join("two.frm"), &frm).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("three.frm"), &frm).unwrap();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // Mode fichier
    let out1 = dir.join("out1");
    let code = cli::run(&s(&["convert", dir.join("one.frm").to_str().unwrap(), "-f", "bmp", "-d", "24", "-o", out1.to_str().unwrap(), "-n", "renamed"]));
    assert_eq!(code, 0);
    assert!(out1.join("renamed.bmp").exists());
    // Mode répertoire (non récursif) avec palette 8 bits Fallout
    let out2 = dir.join("out2");
    assert_eq!(cli::run(&s(&["convert", dir.to_str().unwrap(), "--format", "gif", "-d", "8f", "-o", out2.to_str().unwrap()])), 0);
    assert!(out2.join("one.gif").exists() && out2.join("two.gif").exists() && !out2.join("three.gif").exists());
    // Récursif
    let out3 = dir.join("out3");
    assert_eq!(cli::run(&s(&["convert", dir.to_str().unwrap(), "-f", "tga", "-r", "-o", out3.to_str().unwrap()])), 0);
    assert!(out3.join("sub").join("three.tga").exists());
    // Archive DAT : toute l'archive
    let arc = dat::write_dat2(&[(r"art\x\a.frm", &frm, true), (r"art\b.frm", &frm, false), ("n.txt", b"x", false)]);
    std::fs::write(dir.join("t.dat"), arc).unwrap();
    let out4 = dir.join("out4");
    assert_eq!(cli::run(&s(&["convert", dir.join("t.dat").to_str().unwrap(), "-f", "bmp", "-o", out4.to_str().unwrap()])), 0);
    assert!(out4.join("art").join("x").join("a.bmp").exists() && out4.join("art").join("b.bmp").exists());
    // Erreurs d'usage
    assert_eq!(cli::run(&s(&["convert", dir.to_str().unwrap()])), 2);
    assert_eq!(cli::run(&s(&["convert", dir.to_str().unwrap(), "-f", "jpg", "-d", "8"])), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lzss_back_references() {
    // « abc » en octets bruts puis une référence de 9 octets vers la position 4078 de l'anneau.
    let mut out = Vec::new();
    dat::lzss(&[0x07, b'a', b'b', b'c', 0xEE, 0xF6], &mut out);
    assert_eq!(out, b"abcabcabcabc");
    // Les espaces d'initialisation de l'anneau sont lisibles (position 0, longueur 3).
    let mut out = Vec::new();
    dat::lzss(&[0x00, 0x00, 0x00], &mut out);
    assert_eq!(out, b"   ");
}

#[test]
fn create_pal_next_to_image() {
    use fallout_image_browser::convert::*;
    use fallout_image_browser::formats::EncodeOpts;
    use fallout_image_browser::loader::{PalPolicy, load_file};
    use fallout_image_browser::model::{Picture, Pixels};
    let dir = tmp("pal");
    // Dégradé couleur avec un coin transparent, écrit en TGA 32 bits.
    let (w, h) = (48usize, 32usize);
    let mut v = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            v[o] = (x * 5) as u8;
            v[o + 1] = (y * 7) as u8;
            v[o + 2] = ((x + y) * 3) as u8;
            if x < 6 && y < 6 {
                v[o + 3] = 0;
            }
        }
    }
    let pic = Picture::single(Format::Tga, w as u32, h as u32, Pixels::Rgba(v.clone()), None);
    let o = ConvertOptions { format: Format::Tga, depth: DepthChoice::Rgba32, dither: false, bg: [0, 0, 0], enc: EncodeOpts::default(), ops: Default::default() };
    std::fs::write(dir.join("src.tga"), convert_to_bytes(&pic, &o, &Selection::default()).unwrap()).unwrap();
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let out = dir.join("out");
    let run = || cli::run(&s(&["convert", dir.join("src.tga").to_str().unwrap(), "-f", "frm", "-d", "8p", "-o", out.to_str().unwrap()]));
    assert_eq!(run(), 0);
    let pal = std::fs::read(out.join("src.pal")).unwrap();
    assert_eq!(pal.len(), 768);
    assert!(pal.iter().all(|&b| b <= 63), "palette 6 bits");
    // La palette du même nom est reprise automatiquement à la lecture du FRM.
    let l = load_file(&out.join("src.frm"), &PalPolicy::Auto).unwrap();
    assert!(l.pic.has_transparency, "l'index 0 reste transparent");
    let rgba = fallout_image_browser::model::frame_to_rgba(&l.pic.frames[0], l.pic.palette.as_ref());
    let (mut err, mut n) = (0u64, 0u64);
    for i in 0..w * h {
        if v[i * 4 + 3] == 255 {
            assert_eq!(rgba[i * 4 + 3], 255);
            for c in 0..3 {
                err += (v[i * 4 + c] as i32 - rgba[i * 4 + c] as i32).unsigned_abs() as u64;
                n += 1;
            }
        } else {
            assert_eq!(rgba[i * 4 + 3], 0);
        }
    }
    assert!((err as f64 / n as f64) < 12.0, "erreur moyenne {}", err as f64 / n as f64);
    // Une seconde conversion ne remplace ni l'image ni la palette.
    assert_eq!(run(), 0);
    assert!(out.join("src_converted.frm").exists() && out.join("src_converted.pal").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_help_and_selection_options() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    for h in ["/?", "-?", "-h", "--help", "help", "--formats", "--version"] {
        assert!(cli::is_cli(&s(&[h])), "{h}");
        assert_eq!(cli::run(&s(&[h])), 0, "{h}");
    }
    assert!(!cli::is_cli(&s(&[r"D:\some\folder"])));
    assert_eq!(cli::run(&s(&["convert", "x", "-h"])), 0);
    // Image courante / direction : critter à 6 directions et 12 images.
    let src = sample("fallout/HMJMPSAA.FRM");
    let src = src.to_str().unwrap();
    let dir = tmp("sel");
    let out = dir.join("o");
    let o = out.to_str().unwrap();
    assert_eq!(cli::run(&s(&["convert", src, "-f", "bmp", "-d", "24", "--direction", "3", "--frame", "5", "-o", o, "-q"])), 0);
    assert_eq!(cli::run(&s(&["convert", src, "-f", "bmp", "--sequence", "-n", "seq", "-o", o, "-q"])), 0);
    let names: Vec<String> = std::fs::read_dir(&out).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert!(names.contains(&"HMJMPSAA.bmp".to_string()));
    assert_eq!(names.iter().filter(|n| n.starts_with("seq_d")).count(), 72, "{names:?}");
    assert_eq!(cli::run(&s(&["convert", src, "-f", "bmp", "--direction", "0"])), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un nom d'entrée ne doit jamais pouvoir désigner un fichier hors du dossier de sortie (`..`, lecteur, chemin absolu).
#[test]
fn hostile_entry_names_stay_inside_output_folder() {
    let base = tmp("traversal");
    let frm = sample_frm();
    let arc = dat::write_dat2(&[
        (r"..\..\evil1.frm", &frm, false),
        (r"C:\evil2.frm", &frm, false),
        (r"a\..\..\evil3.frm", &frm, false),
        (r"/abs/evil4.frm", &frm, false),
        (r"ok\fine.frm", &frm, false),
    ]);
    std::fs::write(base.join("t.dat"), arc).unwrap();
    let out = base.join("deep").join("out");
    let args: Vec<String> = ["convert", base.join("t.dat").to_str().unwrap(), "-f", "bmp", "-o", out.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    cli::run(&args);
    // Rien d'écrit en dehors de `out`.
    let mut outside = Vec::new();
    fn scan(dir: &std::path::Path, out: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p == *out {
                continue;
            }
            if p.is_dir() {
                scan(&p, out, found);
            } else if p.extension().map(|x| x == "bmp").unwrap_or(false) {
                found.push(p);
            }
        }
    }
    scan(&base, &out, &mut outside);
    let c_drive = std::path::Path::new(r"C:\evil2.bmp");
    assert!(outside.is_empty(), "fichiers hors du dossier de sortie : {outside:?}");
    assert!(!c_drive.exists(), "écriture à la racine du disque");
    assert!(out.join("ok").join("fine.bmp").exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// Tailles d'entrée mensongères (4 Go annoncés) : erreur propre, sans allocation démesurée.
#[test]
fn lying_entry_sizes_are_rejected() {
    let base = tmp("lying");
    let mut arc = dat::write_dat2(&[("a.frm", b"hello world", false)]);
    let tree_size = u32::from_le_bytes(arc[arc.len() - 8..arc.len() - 4].try_into().unwrap()) as usize;
    let tree = arc.len() - 8 - tree_size;
    // count(4) + len(4) + "a.frm"(5) + type(1) + size(4) + packed(4) : on écrase `size` puis `packed`.
    let size_at = tree + 4 + 4 + 5 + 1;
    arc[size_at..size_at + 8].copy_from_slice(&[0xF0, 0xFF, 0xFF, 0xFF, 0xF0, 0xFF, 0xFF, 0xFF]);
    std::fs::write(base.join("l.dat"), arc).unwrap();
    let d = dat::Dat::open(&base.join("l.dat")).unwrap();
    assert!(d.read(0).is_err());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn extract_file_folder_and_whole_archive() {
    let base = tmp("extract");
    let frm = sample_frm();
    let arc = dat::write_dat2(&[(r"art\intrface\a.frm", &frm, true), (r"art\intrface\sub\b.frm", &frm, false), (r"art\misc\c.frm", &frm, true), ("readme.txt", b"hello", false)]);
    let dat_path = base.join("master.dat");
    std::fs::write(&dat_path, arc).unwrap();

    // Un fichier.
    let one = base.join("one");
    let r = dat::extract(&dat_path.join("art").join("misc").join("c.frm"), &one, false).unwrap();
    assert_eq!((r.written, r.skipped), (1, 0));
    assert_eq!(std::fs::read(one.join("c.frm")).unwrap(), frm);

    // Un dossier : l'arborescence est relative au dossier extrait.
    let folder = base.join("folder");
    let r = dat::extract(&dat_path.join("art").join("intrface"), &folder, false).unwrap();
    assert_eq!((r.written, r.skipped), (2, 0));
    assert_eq!(std::fs::read(folder.join("a.frm")).unwrap(), frm);
    assert_eq!(std::fs::read(folder.join("sub").join("b.frm")).unwrap(), frm);

    // Toute l'archive ; un second passage ne remplace rien, sauf avec `overwrite`.
    let all = base.join("all");
    assert_eq!(dat::extract(&dat_path, &all, false).unwrap().written, 4);
    assert_eq!(std::fs::read(all.join("readme.txt")).unwrap(), b"hello");
    std::fs::write(all.join("readme.txt"), b"mine").unwrap();
    let again = dat::extract(&dat_path, &all, false).unwrap();
    assert_eq!((again.written, again.skipped), (0, 4));
    assert_eq!(std::fs::read(all.join("readme.txt")).unwrap(), b"mine");
    assert_eq!(dat::extract(&dat_path, &all, true).unwrap().written, 4);
    assert_eq!(std::fs::read(all.join("readme.txt")).unwrap(), b"hello");

    // Chemin absent de l'archive.
    assert!(dat::extract(&dat_path.join("nope"), &base.join("x"), false).is_err());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cli_extract_subcommand() {
    let base = tmp("cliextract");
    let frm = sample_frm();
    std::fs::write(base.join("pack.dat"), dat::write_dat2(&[(r"art\x\a.frm", &frm, true), (r"art\b.frm", &frm, false)])).unwrap();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let out = base.join("out");
    let dat_s = base.join("pack.dat");
    assert_eq!(cli::run(&s(&["extract", dat_s.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"])), 0);
    assert_eq!(std::fs::read(out.join("art").join("x").join("a.frm")).unwrap(), frm);
    assert_eq!(std::fs::read(out.join("art").join("b.frm")).unwrap(), frm);
    // Dossier de l'archive seulement.
    let sub = base.join("sub");
    assert_eq!(cli::run(&s(&["extract", base.join("pack.dat").join("art").join("x").to_str().unwrap(), "-o", sub.to_str().unwrap(), "-q"])), 0);
    assert!(sub.join("a.frm").exists() && !sub.join("b.frm").exists());
    // Erreurs d'usage : pas d'entrée, entrée hors archive, option inconnue.
    assert_eq!(cli::run(&s(&["extract"])), 2);
    assert_eq!(cli::run(&s(&["extract", base.to_str().unwrap()])), 2);
    assert_eq!(cli::run(&s(&["extract", dat_s.to_str().unwrap(), "--bogus"])), 2);
    assert!(cli::is_cli(&s(&["extract", "x"])));
    let _ = std::fs::remove_dir_all(&base);
}

/// Un nom d'entrée piégé ne peut pas faire sortir l'extraction de son dossier.
#[test]
fn hostile_names_do_not_escape_extraction() {
    let base = tmp("extractevil");
    let arc = dat::write_dat2(&[(r"..\..\evil.txt", b"x", false), (r"C:\evil2.txt", b"x", false), (r"ok\fine.txt", b"x", false)]);
    std::fs::write(base.join("t.dat"), arc).unwrap();
    let out = base.join("deep").join("out");
    dat::extract(&base.join("t.dat"), &out, false).unwrap();
    assert!(out.join("ok").join("fine.txt").exists());
    assert!(!base.join("evil.txt").exists() && !base.join("deep").join("evil.txt").exists());
    assert!(!Path::new(r"C:\evil2.txt").exists());
    let _ = std::fs::remove_dir_all(&base);
}
