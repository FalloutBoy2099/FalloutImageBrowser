//! Arborescence des dossiers (chargement paresseux) et liste des images du dossier courant.

use crate::model::is_supported;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
}

pub struct DirNode {
    pub path: PathBuf,
    pub name: String,
    /// Archive .DAT (fichier) : seul cas affiché en bleu ; un dossier nommé « x.dat » reste un dossier.
    pub archive: bool,
    pub expanded: bool,
    pub loaded: bool,
    pub children: Vec<DirNode>,
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Dir { expanded: bool },
    File,
}

#[derive(Clone)]
pub struct Row {
    pub archive: bool,
    pub depth: usize,
    pub kind: RowKind,
    pub path: PathBuf,
    pub name: String,
}

pub struct Tree {
    pub roots: Vec<DirNode>,
    pub rows: Vec<Row>,
    pub selected_dir: Option<PathBuf>,
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ia, mut ib) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                let mut nb = String::new();
                while let Some(&c) = ia.peek() {
                    if c.is_ascii_digit() {
                        na.push(c);
                        ia.next();
                    } else {
                        break;
                    }
                }
                while let Some(&c) = ib.peek() {
                    if c.is_ascii_digit() {
                        nb.push(c);
                        ib.next();
                    } else {
                        break;
                    }
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ia.next();
                ib.next();
            }
        }
    }
}

#[cfg(windows)]
fn is_hidden(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::metadata(p).map(|m| m.file_attributes() & 0x2 != 0 || m.file_attributes() & 0x4 != 0).unwrap_or(false)
}
#[cfg(not(windows))]
fn is_hidden(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false)
}

#[cfg(windows)]
fn root_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let mask = unsafe { GetLogicalDrives() };
    (0..26).filter(|i| mask & (1 << i) != 0).map(|i| PathBuf::from(format!("{}:\\", (b'A' + i as u8) as char))).collect()
}
#[cfg(not(windows))]
fn root_paths() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/")];
    if let Some(h) = std::env::var_os("HOME") {
        v.insert(0, PathBuf::from(h));
    }
    v
}

impl DirNode {
    fn new(path: PathBuf, name: String) -> DirNode {
        DirNode { path, name, archive: false, expanded: false, loaded: false, children: Vec::new(), files: Vec::new() }
    }

    pub fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.children.clear();
        self.files.clear();
        // Archive .DAT (ou dossier à l'intérieur) : contenu virtuel.
        if let Some((arch, inner)) = crate::dat::split(&self.path) {
            if let Ok(dat) = crate::dat::Dat::open(&arch) {
                let (dirs, files) = dat.children(&inner);
                for d in dirs {
                    self.children.push(DirNode::new(self.path.join(&d), d));
                }
                for e in files {
                    let p = self.path.join(e.name());
                    if is_supported(&p) {
                        self.files.push(FileEntry { path: p, name: e.name().to_string() });
                    }
                }
            }
            self.children.sort_by(|a, b| natural_cmp(&a.name, &b.name));
            self.files.sort_by(|a, b| natural_cmp(&a.name, &b.name));
            return;
        }
        let Ok(rd) = std::fs::read_dir(&self.path) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
                if name.starts_with('$') || name == "System Volume Information" || is_hidden(&p) {
                    continue;
                }
                self.children.push(DirNode::new(p, name));
            } else if crate::dat::is_dat_ext(&p) {
                let mut n = DirNode::new(p, name);
                n.archive = true;
                self.children.push(n);
            } else if is_supported(&p) {
                self.files.push(FileEntry { path: p, name });
            }
        }
        self.children.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        self.files.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    }

    fn reload(&mut self) {
        let expanded: Vec<(PathBuf, bool)> = self.children.iter().map(|c| (c.path.clone(), c.expanded)).collect();
        self.loaded = false;
        self.load();
        for c in &mut self.children {
            if let Some((_, e)) = expanded.iter().find(|(p, _)| *p == c.path) {
                c.expanded = *e;
            }
        }
    }
}

impl Tree {
    pub fn new() -> Tree {
        let roots = root_paths()
            .into_iter()
            .map(|p| {
                let name = p.display().to_string().trim_end_matches(['\\', '/']).to_string();
                let name = if name.is_empty() { "/".to_string() } else { name };
                DirNode::new(p, name)
            })
            .collect();
        let mut t = Tree { roots, rows: Vec::new(), selected_dir: None };
        t.rebuild();
        t
    }

    fn find_mut<'a>(nodes: &'a mut [DirNode], path: &Path) -> Option<&'a mut DirNode> {
        for n in nodes.iter_mut() {
            if n.path == path {
                return Some(n);
            }
            if path.starts_with(&n.path) {
                n.load();
                if let Some(f) = Self::find_mut(&mut n.children, path) {
                    return Some(f);
                }
            }
        }
        None
    }

    fn find<'a>(nodes: &'a [DirNode], path: &Path) -> Option<&'a DirNode> {
        for n in nodes {
            if n.path == path {
                return Some(n);
            }
            if path.starts_with(&n.path) {
                if let Some(f) = Self::find(&n.children, path) {
                    return Some(f);
                }
            }
        }
        None
    }

    /// Développe tous les ancêtres de `dir`, le sélectionne et charge son contenu.
    pub fn reveal_dir(&mut self, dir: &Path) {
        let mut chain: Vec<PathBuf> = dir.ancestors().map(|p| p.to_path_buf()).collect();
        chain.reverse();
        for p in &chain {
            if p.as_os_str().is_empty() {
                continue;
            }
            if let Some(n) = Self::find_mut(&mut self.roots, p) {
                n.load();
                n.expanded = true;
            }
        }
        self.selected_dir = Some(dir.to_path_buf());
        self.rebuild();
    }

    pub fn toggle(&mut self, path: &Path) {
        if let Some(n) = Self::find_mut(&mut self.roots, path) {
            n.load();
            n.expanded = !n.expanded;
        }
        self.rebuild();
    }

    pub fn select_dir(&mut self, path: &Path) {
        if let Some(n) = Self::find_mut(&mut self.roots, path) {
            n.load();
            n.expanded = true;
        }
        self.selected_dir = Some(path.to_path_buf());
        self.rebuild();
    }

    pub fn reload_dir(&mut self, path: &Path) {
        if let Some(n) = Self::find_mut(&mut self.roots, path) {
            n.reload();
        }
        self.rebuild();
    }

    pub fn files_of(&self, dir: &Path) -> Vec<PathBuf> {
        Self::find(&self.roots, dir).map(|n| n.files.iter().map(|f| f.path.clone()).collect()).unwrap_or_default()
    }

    pub fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let sel = self.selected_dir.clone();
        fn walk(n: &DirNode, depth: usize, sel: &Option<PathBuf>, rows: &mut Vec<Row>) {
            rows.push(Row {
                archive: n.archive,
                depth,
                kind: RowKind::Dir { expanded: n.expanded },
                path: n.path.clone(),
                name: n.name.clone(),
            });
            if n.expanded {
                for c in &n.children {
                    walk(c, depth + 1, sel, rows);
                }
                if sel.as_deref() == Some(n.path.as_path()) {
                    for f in &n.files {
                        rows.push(Row { archive: false, depth: depth + 1, kind: RowKind::File, path: f.path.clone(), name: f.name.clone() });
                    }
                }
            }
        }
        for r in &self.roots {
            walk(r, 0, &sel, &mut rows);
        }
        self.rows = rows;
    }
}

impl DirNode {
    /// Compte les sous-dossiers (récursivement, en chargeant le contenu) en s'arrêtant au-delà de `limit`.
    fn count_dirs(&mut self, limit: usize, acc: &mut usize) {
        self.load();
        for c in &mut self.children {
            *acc += 1;
            if *acc > limit {
                return;
            }
            c.count_dirs(limit, acc);
            if *acc > limit {
                return;
            }
        }
    }

    fn expand_all(&mut self, budget: &mut usize) {
        self.load();
        self.expanded = true;
        for c in &mut self.children {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            c.expand_all(budget);
        }
    }

    fn collapse_subtree(&mut self) {
        self.expanded = false;
        for c in &mut self.children {
            c.collapse_subtree();
        }
    }

    /// Replie tout ce qui n'est ni un ancêtre, ni l'élément `target`, ni l'un de ses descendants.
    fn collapse_others(&mut self, target: &Path) {
        if self.path == target || self.path.starts_with(target) {
            return; // l'élément et ses descendants gardent leur état
        }
        if target.starts_with(&self.path) {
            for c in &mut self.children {
                c.collapse_others(target); // ancêtre : reste déplié
            }
        } else {
            self.collapse_subtree();
        }
    }
}

impl Tree {
    /// Nombre de sous-dossiers de `path` (plafonné à `limit + 1`).
    pub fn count_dirs(&mut self, path: &Path, limit: usize) -> usize {
        let mut n = 0;
        if let Some(node) = Self::find_mut(&mut self.roots, path) {
            node.count_dirs(limit, &mut n);
        }
        n
    }

    /// Déplie tous les sous-dossiers de `path` (plafond de sécurité : 20 000 dossiers).
    pub fn expand_all(&mut self, path: &Path) {
        let mut budget = 20_000usize;
        if let Some(node) = Self::find_mut(&mut self.roots, path) {
            node.expand_all(&mut budget);
        }
        self.rebuild();
    }

    pub fn collapse_others(&mut self, path: &Path) {
        for r in &mut self.roots {
            r.collapse_others(path);
        }
        self.rebuild();
    }
}
