//! Tri et filtre des fichiers d'un dossier.

use fallout_image_browser::tree::{FileEntry, RowKind, SortKey, Tree, ordered_files};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

fn entry(name: &str, size: u64, age_secs: u64) -> FileEntry {
    FileEntry { path: PathBuf::from(name), name: name.to_string(), size, modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs)) }
}

fn names(v: Vec<&FileEntry>) -> Vec<&str> {
    v.into_iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn sorting_and_filtering_rules() {
    let files = vec![entry("img10.bmp", 300, 10), entry("img2.bmp", 100, 30), entry("Map.frm", 200, 20), entry("a.png", 100, 0)];
    // Nom : ordre naturel, sans tenir compte de la casse (img2 avant img10).
    assert_eq!(names(ordered_files(&files, SortKey::Name, false, "")), ["a.png", "img2.bmp", "img10.bmp", "Map.frm"]);
    assert_eq!(names(ordered_files(&files, SortKey::Name, true, "")), ["Map.frm", "img10.bmp", "img2.bmp", "a.png"]);
    // Taille : les égalités sont départagées par le nom.
    assert_eq!(names(ordered_files(&files, SortKey::Size, false, "")), ["a.png", "img2.bmp", "Map.frm", "img10.bmp"]);
    // Date : du plus ancien au plus récent.
    assert_eq!(names(ordered_files(&files, SortKey::Modified, false, "")), ["img2.bmp", "Map.frm", "img10.bmp", "a.png"]);
    // Type : par extension, puis par nom.
    assert_eq!(names(ordered_files(&files, SortKey::Type, false, "")), ["img2.bmp", "img10.bmp", "Map.frm", "a.png"]);
    // Filtre : sous-chaîne, insensible à la casse, espaces autour ignorés.
    assert_eq!(names(ordered_files(&files, SortKey::Name, false, "  IMG ")), ["img2.bmp", "img10.bmp"]);
    assert_eq!(names(ordered_files(&files, SortKey::Name, false, ".frm")), ["Map.frm"]);
    assert!(ordered_files(&files, SortKey::Name, false, "zzz").is_empty());
}

#[test]
fn tree_applies_sort_and_filter_to_the_current_folder() {
    let dir = std::env::temp_dir().join(format!("fib_tree_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("b.bmp"), vec![0u8; 50]).unwrap();
    std::fs::write(dir.join("a10.png"), vec![0u8; 10]).unwrap();
    std::fs::write(dir.join("a2.png"), vec![0u8; 30]).unwrap();
    std::fs::write(dir.join("notes.txt"), b"not an image").unwrap();

    let mut tree = Tree::new();
    tree.reveal_dir(&dir);
    tree.select_dir(&dir);
    let base = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>();
    assert_eq!(base(tree.files_of(&dir)), ["a2.png", "a10.png", "b.bmp"], "seules les images sont listées");

    tree.sort = SortKey::Size;
    tree.rebuild();
    assert_eq!(base(tree.files_of(&dir)), ["a10.png", "a2.png", "b.bmp"]);
    tree.descending = true;
    tree.rebuild();
    assert_eq!(base(tree.files_of(&dir)), ["b.bmp", "a2.png", "a10.png"]);

    tree.filter = "a".to_string();
    tree.rebuild();
    assert_eq!(base(tree.files_of(&dir)), ["a2.png", "a10.png"]);
    // Les lignes de l'arborescence suivent le même ordre.
    let rows: Vec<String> = tree.rows.iter().filter(|r| r.kind == RowKind::File && r.name.ends_with(".png")).map(|r| r.name.clone()).collect();
    assert_eq!(rows, ["a2.png", "a10.png"]);
    let _ = std::fs::remove_dir_all(&dir);
}
