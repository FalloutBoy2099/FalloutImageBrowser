//! Copie, déplacement et renommage du fichier affiché.

use fallout_image_browser::{dat, fileops};
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fib_fileops_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn names_are_validated() {
    for ok in ["a.frm", "image 2.png", "é à.bmp", "no_extension", ".hidden"] {
        assert!(fileops::validate_name(ok).is_ok(), "{ok}");
    }
    for bad in ["", "  ", ".", "..", "a/b.png", "a\\b.png", "c:x.png", "a*.png", "what?.png", "q\"uote", "<x>", "pipe|", "tab\t.png", "end.", "trail "] {
        assert!(fileops::validate_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn free_paths_never_overwrite() {
    let d = tmp("free");
    assert_eq!(fileops::free_path(&d, "a.png"), d.join("a.png"));
    std::fs::write(d.join("a.png"), b"1").unwrap();
    assert_eq!(fileops::free_path(&d, "a.png"), d.join("a (2).png"));
    std::fs::write(d.join("a (2).png"), b"2").unwrap();
    assert_eq!(fileops::free_path(&d, "a.png"), d.join("a (3).png"));
    std::fs::write(d.join("noext"), b"3").unwrap();
    assert_eq!(fileops::free_path(&d, "noext"), d.join("noext (2)"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn copy_keeps_the_original_and_the_palette() {
    let d = tmp("copy");
    let (src_dir, out) = (d.join("src"), d.join("out"));
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    let frm = src_dir.join("HERO.FRM");
    std::fs::write(&frm, b"frm data").unwrap();
    std::fs::write(src_dir.join("HERO.pal"), b"pal data").unwrap();

    let copy = fileops::copy_to(&frm, &out).unwrap();
    assert_eq!(copy, out.join("HERO.FRM"));
    assert_eq!((read(&copy), read(&out.join("HERO.pal"))), (b"frm data".to_vec(), b"pal data".to_vec()));
    assert!(frm.exists() && src_dir.join("HERO.pal").exists(), "l'original reste en place");
    // Une seconde copie reçoit un autre nom, palette comprise, et n'écrase rien.
    std::fs::write(out.join("HERO.FRM"), b"mine").unwrap();
    let second = fileops::copy_to(&frm, &out).unwrap();
    assert_eq!(second, out.join("HERO (2).FRM"));
    assert_eq!(read(&out.join("HERO.FRM")), b"mine");
    assert_eq!(read(&out.join("HERO (2).pal")), b"pal data");
    assert!(fileops::copy_to(&frm, &d.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn move_takes_the_palette_along_and_refuses_the_same_folder() {
    let d = tmp("move");
    let (src_dir, out) = (d.join("src"), d.join("out"));
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    let frm = src_dir.join("a.frm");
    std::fs::write(&frm, b"x").unwrap();
    std::fs::write(src_dir.join("a.PAL"), b"p").unwrap();
    assert!(fileops::move_to(&frm, &src_dir).is_err(), "même dossier");
    assert!(frm.exists());
    let moved = fileops::move_to(&frm, &out).unwrap();
    assert_eq!(read(&moved), b"x");
    assert!(!frm.exists() && !src_dir.join("a.PAL").exists());
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2, "image et palette sont dans le dossier de destination");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rename_checks_the_name_and_moves_the_palette() {
    let d = tmp("rename");
    let a = d.join("a.frm");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(d.join("a.pal"), b"pal").unwrap();
    std::fs::write(d.join("taken.frm"), b"t").unwrap();
    assert!(fileops::rename(&a, "taken.frm").is_err(), "nom déjà pris");
    assert!(fileops::rename(&a, "bad/name.frm").is_err());
    assert!(fileops::rename(&a, "").is_err());
    assert!(a.exists());
    let b = fileops::rename(&a, "  renamed.frm ").unwrap();
    assert_eq!(b, d.join("renamed.frm"));
    assert_eq!(read(&b), b"a");
    assert!(!a.exists() && !d.join("a.pal").exists());
    assert_eq!(read(&d.join("renamed.pal")), b"pal");
    // Changer seulement la casse du même fichier est permis.
    let c = fileops::rename(&b, "RENAMED.frm").unwrap();
    assert_eq!(c.file_name().unwrap(), "RENAMED.frm");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn archive_items_can_be_copied_but_not_moved_or_renamed() {
    let d = tmp("archive");
    let arc = dat::write_dat2(&[(r"art\x\a.frm", b"inside", true)]);
    std::fs::write(d.join("pack.dat"), arc).unwrap();
    let inner = d.join("pack.dat").join("art").join("x").join("a.frm");
    let out = d.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let copy = fileops::copy_to(&inner, &out).unwrap();
    assert_eq!(read(&copy), b"inside");
    assert!(fileops::move_to(&inner, &out).is_err());
    assert!(fileops::rename(&inner, "b.frm").is_err());
    let _ = std::fs::remove_dir_all(&d);
}
