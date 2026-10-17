//! Archives .ZIP et .7Z lues comme des dossiers virtuels (comme les .DAT).

use fallout_image_browser::dat::{self, Stat};
use fallout_image_browser::loader::{PalPolicy, load_file};
use fallout_image_browser::tree::Tree;
use std::path::PathBuf;

fn sample(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("_test_files").join("archives").join(name)
}

#[test]
fn zip_and_7z_behave_like_dat_folders() {
    for name in ["sample.zip", "sample.7z"] {
        let arch = sample(name);
        assert_eq!(dat::stat(&arch), Some(Stat::Dir), "{name}");
        assert!(dat::is_virtual(&arch));
        assert_eq!(dat::stat(&arch.join("sub")), Some(Stat::Dir), "{name}");
        assert_eq!(dat::stat(&arch.join("sub").join("b.ppm")), Some(Stat::File), "{name}");
        assert_eq!(dat::stat(&arch.join("missing.ppm")), None);
        assert_eq!(dat::read_any(&arch.join("notes.txt")).unwrap(), b"hello");
        let l = load_file(&arch.join("sub").join("b.ppm"), &PalPolicy::Fallout).unwrap();
        assert_eq!((l.pic.frames[0].width, l.pic.frames[0].height), (6, 3), "{name}");
        // Recherche insensible à la casse, comme pour un .DAT.
        assert!(dat::exists_any(&arch.join("SUB").join("B.PPM")));
        // Extraction (récursive) : l'arborescence est conservée.
        let out = std::env::temp_dir().join(format!("fib_arch_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let r = dat::extract(&arch, &out, false).unwrap();
        assert_eq!(r.written, 3, "{name}");
        assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"hello");
        assert!(out.join("sub").join("b.ppm").is_file());
        let _ = std::fs::remove_dir_all(&out);
    }
}

#[test]
fn archives_appear_as_blue_folders_in_the_tree() {
    let dir = sample("sample.zip").parent().unwrap().to_path_buf();
    let mut t = Tree::new();
    t.reveal_dir(&dir);
    let rows: Vec<_> = t.rows.iter().filter(|r| r.archive).map(|r| r.name.clone()).collect();
    assert!(rows.contains(&"sample.zip".to_string()) && rows.contains(&"sample.7z".to_string()), "{rows:?}");
}

#[test]
fn corrupt_archives_report_an_error() {
    let dir = std::env::temp_dir().join(format!("fib_badarch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["bad.zip", "bad.7z"] {
        std::fs::write(dir.join(name), b"this is not an archive at all").unwrap();
        assert_eq!(dat::stat(&dir.join(name).join("x.png")), None);
        assert!(dat::read_any(&dir.join(name).join("x.png")).is_err());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
