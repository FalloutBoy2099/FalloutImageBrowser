//! Archives .DAT de Fallout 1 (DAT1, LZSS) et Fallout 2 (DAT2, zlib), ainsi que .ZIP et .7Z, lues comme des dossiers virtuels.
//!
//! Un fichier contenu dans une archive est désigné par un chemin « normal » :
//! `C:\Jeux\master.dat\art\critters\hmjmpsaa.frm`. Toutes les fonctions du programme qui prennent un
//! `Path` passent par [`read_any`] / [`exists_any`] et fonctionnent donc sans traitement particulier.

use anyhow::{Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DatKind {
    Dat1,
    Dat2,
    /// Archive ZIP : `DatEntry::offset` est l'index de l'entrée.
    Zip,
    /// Archive 7z : `DatEntry::offset` est l'index du fichier dans l'archive.
    SevenZ,
}

#[derive(Clone, Debug)]
pub struct DatEntry {
    /// Chemin dans l'archive, séparateur `/`, casse d'origine.
    pub path: String,
    pub flags: u32,
    pub offset: u64,
    pub size: u32,
    pub packed: u32,
}

impl DatEntry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Default)]
struct DirIndex {
    subdirs: Vec<String>,
    files: Vec<usize>,
}

pub struct Dat {
    pub path: PathBuf,
    pub kind: DatKind,
    pub entries: Vec<DatEntry>,
    lookup: HashMap<String, usize>,
    dirs: HashMap<String, DirIndex>,
}

/// Chemin interne normalisé (séparateur `/`). Les composants `.`, `..` et vides sont supprimés et `:` remplacé :
/// un nom d'entrée ne peut donc pas désigner un chemin hors du dossier de sortie lors d'une extraction ou d'une conversion.
fn norm(s: &str) -> String {
    s.replace('\\', "/").split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").map(|c| c.replace(':', "_")).collect::<Vec<_>>().join("/")
}

/// Taille maximale d'une entrée décompressée (protège d'une archive piégée).
const MAX_ENTRY: u64 = 1 << 30;

fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<Dat>>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Arc<Dat>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn is_dat_ext(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("dat")).unwrap_or(false)
}

/// Extension d'une archive lue comme un dossier : .dat, .zip ou .7z.
pub fn is_archive_ext(p: &Path) -> bool {
    p.extension().map(|e| ["dat", "zip", "7z"].iter().any(|x| e.eq_ignore_ascii_case(x))).unwrap_or(false)
}

/// Découpe `…/master.dat/art/x.frm` en (archive, chemin interne `art/x.frm`).
pub fn split(path: &Path) -> Option<(PathBuf, String)> {
    for a in path.ancestors() {
        if is_archive_ext(a) && a.is_file() {
            let rest = path.strip_prefix(a).ok()?;
            let inner: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            return Some((a.to_path_buf(), inner.join("/")));
        }
    }
    None
}

/// Le chemin désigne-t-il l'intérieur d'une archive (ou l'archive elle-même) ?
pub fn is_virtual(path: &Path) -> bool {
    split(path).is_some()
}

#[derive(PartialEq, Eq, Debug)]
pub enum Stat {
    File,
    Dir,
}

/// État d'un chemin virtuel : fichier, dossier (y compris la racine de l'archive) ou inexistant.
pub fn stat(path: &Path) -> Option<Stat> {
    let (arch, inner) = split(path)?;
    let dat = Dat::open(&arch).ok()?;
    if inner.is_empty() {
        return Some(Stat::Dir);
    }
    let key = inner.to_lowercase();
    if dat.lookup.contains_key(&key) {
        Some(Stat::File)
    } else if dat.dirs.contains_key(&key) {
        Some(Stat::Dir)
    } else {
        None
    }
}

pub fn read_any(path: &Path) -> Result<Vec<u8>> {
    if let Some((arch, inner)) = split(path) {
        let dat = Dat::open(&arch)?;
        let idx = dat.find(&inner).ok_or_else(|| anyhow!("{} : absent de l'archive", path.display()))?;
        return dat.read(idx);
    }
    std::fs::read(path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

pub fn exists_any(path: &Path) -> bool {
    if let Some((arch, inner)) = split(path) {
        return Dat::open(&arch).map(|d| d.find(&inner).is_some()).unwrap_or(false);
    }
    path.is_file()
}

/// Dossier réel qui contient le fichier (pour une archive : le dossier de l'archive).
pub fn real_parent(path: &Path) -> Option<PathBuf> {
    match split(path) {
        Some((arch, _)) => arch.parent().map(|p| p.to_path_buf()),
        None => path.parent().map(|p| p.to_path_buf()),
    }
}

/// Fichiers (récursifs) d'un dossier virtuel : (chemin complet, chemin relatif au dossier).
pub fn walk(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Some((arch, inner)) = split(dir) else { return Vec::new() };
    let Ok(dat) = Dat::open(&arch) else { return Vec::new() };
    let prefix = if inner.is_empty() { String::new() } else { format!("{}/", inner.to_lowercase()) };
    let mut v = Vec::new();
    for e in &dat.entries {
        if prefix.is_empty() || e.path.to_lowercase().starts_with(&prefix) {
            let rel = &e.path[prefix.len()..];
            let mut full = arch.clone();
            for c in e.path.split('/') {
                full.push(c);
            }
            v.push((full, PathBuf::from(rel.replace('/', std::path::MAIN_SEPARATOR_STR))));
        }
    }
    v
}

impl Dat {
    pub fn open(path: &Path) -> Result<Arc<Dat>> {
        // Le verrou reste pris pendant l'analyse : plusieurs threads qui ouvrent la même archive en même temps ne la lisent qu'une fois.
        let mut c = cache().lock().unwrap();
        if let Some(d) = c.get(path) {
            return Ok(d.clone());
        }
        let dat = Arc::new(Dat::parse(path)?);
        c.insert(path.to_path_buf(), dat.clone());
        Ok(dat)
    }

    pub fn find(&self, inner: &str) -> Option<usize> {
        self.lookup.get(&norm(inner).to_lowercase()).copied()
    }

    /// (sous-dossiers, fichiers) d'un dossier interne (`""` = racine).
    pub fn children(&self, dir: &str) -> (Vec<String>, Vec<&DatEntry>) {
        match self.dirs.get(&norm(dir).to_lowercase()) {
            Some(d) => (d.subdirs.clone(), d.files.iter().map(|&i| &self.entries[i]).collect()),
            None => (Vec::new(), Vec::new()),
        }
    }

    fn build(path: &Path, kind: DatKind, entries: Vec<DatEntry>) -> Dat {
        let mut lookup = HashMap::with_capacity(entries.len());
        let mut dirs: HashMap<String, DirIndex> = HashMap::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        dirs.entry(String::new()).or_default();
        for (i, e) in entries.iter().enumerate() {
            let lower = e.path.to_lowercase();
            lookup.insert(lower.clone(), i);
            let comps: Vec<&str> = e.path.split('/').collect();
            // Dossiers parents.
            let mut parent = String::new();
            for c in &comps[..comps.len() - 1] {
                let child = if parent.is_empty() { c.to_lowercase() } else { format!("{parent}/{}", c.to_lowercase()) };
                if seen.insert((parent.clone(), c.to_lowercase())) {
                    dirs.entry(parent.clone()).or_default().subdirs.push(c.to_string());
                }
                dirs.entry(child.clone()).or_default();
                parent = child;
            }
            dirs.entry(parent).or_default().files.push(i);
        }
        Dat { path: path.to_path_buf(), kind, entries, lookup, dirs }
    }

    fn parse(path: &Path) -> Result<Dat> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "zip" => return guarded(|| Dat::parse_zip(path)),
            "7z" => return guarded(|| Dat::parse_7z(path)),
            _ => {}
        }
        let mut f = std::fs::File::open(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let len = f.metadata()?.len();
        if len < 16 {
            bail!("archive DAT trop courte");
        }
        // DAT2 : pied de page (taille de l'arbre, taille du fichier), petit-boutiste.
        f.seek(SeekFrom::Start(len - 8))?;
        let mut foot = [0u8; 8];
        f.read_exact(&mut foot)?;
        let tree_size = u32::from_le_bytes(foot[0..4].try_into().unwrap()) as u64;
        let data_size = u32::from_le_bytes(foot[4..8].try_into().unwrap()) as u64;
        if data_size == len && tree_size + 8 <= len && tree_size >= 4 {
            let base = len - data_size;
            f.seek(SeekFrom::Start(len - 8 - tree_size))?;
            let mut tree = vec![0u8; tree_size as usize];
            f.read_exact(&mut tree)?;
            let entries = parse_dat2(&tree, base)?;
            return Ok(Dat::build(path, DatKind::Dat2, entries));
        }
        // DAT1 : en-tête gros-boutiste ; l'index est en tête de fichier, on lit de plus en plus large jusqu'à pouvoir l'analyser.
        let mut size = (1u64 << 20).min(len);
        loop {
            f.seek(SeekFrom::Start(0))?;
            let mut head = vec![0u8; size as usize];
            f.read_exact(&mut head)?;
            match parse_dat1(&head) {
                Ok(entries) => return Ok(Dat::build(path, DatKind::Dat1, entries)),
                Err(e) if e.to_string().contains("tronqué") && size < len => size = (size * 4).min(len),
                Err(e) => bail!("{} : archive DAT non reconnue ({e})", path.display()),
            }
        }
    }

    fn parse_zip(path: &Path) -> Result<Dat> {
        let f = std::fs::File::open(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| anyhow!("{} : archive ZIP non reconnue ({e})", path.display()))?;
        let mut entries = Vec::new();
        for i in 0..zip.len() {
            let Ok(file) = zip.by_index_raw(i) else { continue };
            let p = norm(&file.name().map(|n| n.to_string()).unwrap_or_else(|_| String::from_utf8_lossy(file.name_raw()).to_string()));
            if file.is_dir() || p.is_empty() {
                continue;
            }
            entries.push(DatEntry { path: p, flags: 0, offset: i as u64, size: file.size().min(u32::MAX as u64) as u32, packed: 0 });
        }
        Ok(Dat::build(path, DatKind::Zip, entries))
    }

    fn parse_7z(path: &Path) -> Result<Dat> {
        let r = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(|e| anyhow!("{} : archive 7z non reconnue ({e})", path.display()))?;
        let mut entries = Vec::new();
        for (i, f) in r.archive().files.iter().enumerate() {
            let p = norm(f.name());
            if f.is_directory() || f.is_anti_item() || p.is_empty() {
                continue;
            }
            entries.push(DatEntry { path: p, flags: 0, offset: i as u64, size: f.size().min(u32::MAX as u64) as u32, packed: 0 });
        }
        Ok(Dat::build(path, DatKind::SevenZ, entries))
    }

    fn read_zip(&self, e: &DatEntry) -> Result<Vec<u8>> {
        let f = std::fs::File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(f).map_err(|er| anyhow!("{er}"))?;
        let file = zip.by_index(e.offset as usize).map_err(|er| anyhow!("{} : {er}", e.path))?;
        // La taille annoncée n'est pas fiable : capacité prudente et lecture bornée (archive piégée).
        let mut out = Vec::with_capacity((e.size as usize).min(64 << 20));
        file.take(MAX_ENTRY).read_to_end(&mut out).map_err(|er| anyhow!("{} : {er}", e.path))?;
        Ok(out)
    }

    fn read_7z(&self, e: &DatEntry) -> Result<Vec<u8>> {
        let mut r = sevenz_rust2::ArchiveReader::open(&self.path, sevenz_rust2::Password::empty()).map_err(|er| anyhow!("{er}"))?;
        let (name, size) = {
            let f = r.archive().files.get(e.offset as usize).ok_or_else(|| anyhow!("entrée hors de l'archive"))?;
            (f.name().to_string(), f.size())
        };
        if size > MAX_ENTRY {
            bail!("{} : entrée trop grande", e.path);
        }
        r.read_file(&name).map_err(|er| anyhow!("{} : {er}", e.path))
    }

    pub fn read(&self, idx: usize) -> Result<Vec<u8>> {
        let e = &self.entries[idx];
        match self.kind {
            DatKind::Zip => return guarded(|| self.read_zip(e)),
            DatKind::SevenZ => return guarded(|| self.read_7z(e)),
            _ => {}
        }
        let mut f = std::fs::File::open(&self.path)?;
        // Les tailles viennent de l'archive : on vérifie qu'elles tiennent dans le fichier avant d'allouer.
        let flen = f.metadata()?.len();
        let room = flen.checked_sub(e.offset).ok_or_else(|| anyhow!("entrée hors de l'archive"))?;
        let fits = |n: u32| if n as u64 <= room { Ok(n as usize) } else { Err(anyhow!("entrée tronquée ou taille invalide")) };
        // Capacité initiale prudente : la taille annoncée n'est pas fiable.
        let cap = (e.size as usize).min(64 << 20);
        f.seek(SeekFrom::Start(e.offset))?;
        match self.kind {
            DatKind::Dat2 => {
                let mut packed = vec![0u8; fits(e.packed)?];
                f.read_exact(&mut packed)?;
                if e.flags == 0 {
                    return Ok(packed);
                }
                let mut out = Vec::with_capacity(cap);
                flate2::read::ZlibDecoder::new(&packed[..]).take(MAX_ENTRY).read_to_end(&mut out).map_err(|e| anyhow!("zlib : {e}"))?;
                Ok(out)
            }
            DatKind::Zip | DatKind::SevenZ => unreachable!("lu plus haut"),
            DatKind::Dat1 => {
                let kind = if e.flags == 0 { 0x10 } else { e.flags & 0xF0 };
                match kind {
                    0x20 => {
                        let mut out = vec![0u8; fits(e.size)?];
                        f.read_exact(&mut out)?;
                        Ok(out)
                    }
                    0x10 => {
                        let mut packed = vec![0u8; fits(e.packed)?];
                        f.read_exact(&mut packed)?;
                        let mut out = Vec::with_capacity(cap);
                        lzss(&packed, &mut out);
                        Ok(out)
                    }
                    0x40 => {
                        let mut raw = vec![0u8; fits(e.packed)?];
                        f.read_exact(&mut raw)?;
                        let mut out = Vec::with_capacity(cap);
                        let mut i = 0usize;
                        while i + 2 <= raw.len() && out.len() < e.size as usize {
                            let b = u16::from_be_bytes([raw[i], raw[i + 1]]);
                            i += 2;
                            let (n, stored) = ((b & 0x7FFF) as usize, b & 0x8000 != 0);
                            let end = (i + n).min(raw.len());
                            if stored {
                                out.extend_from_slice(&raw[i..end]);
                            } else {
                                lzss(&raw[i..end], &mut out);
                            }
                            i = end;
                        }
                        out.truncate(e.size as usize);
                        Ok(out)
                    }
                    k => bail!("type de compression DAT1 inconnu ({k:#X})"),
                }
            }
        }
    }
}

/// Exécute une lecture d'archive tierce en convertissant un éventuel plantage (archive corrompue) en erreur : un panic sous le
/// verrou du cache d'archives le rendrait inutilisable pour le reste de la session.
fn guarded<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(anyhow!("archive corrompue")))
}

/// LZSS de Fallout 1 : anneau de 4096 octets rempli d'espaces, écriture à partir de 4078,
/// octet de contrôle lu du bit de poids faible vers le fort (1 = octet brut, 0 = référence sur 2 octets).
pub fn lzss(src: &[u8], out: &mut Vec<u8>) {
    let mut ring = [0x20u8; 4096];
    let mut r = 4078usize;
    let mut i = 0usize;
    while i < src.len() {
        let flags = src[i];
        i += 1;
        for bit in 0..8 {
            if i >= src.len() {
                return;
            }
            if flags >> bit & 1 == 1 {
                let c = src[i];
                i += 1;
                out.push(c);
                ring[r] = c;
                r = (r + 1) & 0xFFF;
            } else {
                if i + 1 >= src.len() {
                    return;
                }
                let (b1, b2) = (src[i] as usize, src[i + 1] as usize);
                i += 2;
                let off = b1 | ((b2 & 0xF0) << 4);
                for k in 0..(b2 & 0x0F) + 3 {
                    let c = ring[(off + k) & 0xFFF];
                    out.push(c);
                    ring[r] = c;
                    r = (r + 1) & 0xFFF;
                }
            }
        }
    }
}

/// Compresseur LZSS minimal (octets bruts uniquement) : sert aux tests.
pub fn lzss_literal_only(data: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for chunk in data.chunks(8) {
        o.push(((1u16 << chunk.len()) - 1) as u8);
        o.extend_from_slice(chunk);
    }
    o
}

fn parse_dat2(tree: &[u8], base: u64) -> Result<Vec<DatEntry>> {
    let mut p = 0usize;
    let rd32 = |p: &mut usize| -> Result<u32> {
        let b = tree.get(*p..*p + 4).ok_or_else(|| anyhow!("arbre tronqué"))?;
        *p += 4;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    };
    let count = rd32(&mut p)? as usize;
    let mut v = Vec::with_capacity(count.min(1_000_000));
    for _ in 0..count {
        let nl = rd32(&mut p)? as usize;
        let name = tree.get(p..p + nl).ok_or_else(|| anyhow!("arbre tronqué"))?;
        p += nl;
        let comp = *tree.get(p).ok_or_else(|| anyhow!("arbre tronqué"))?;
        p += 1;
        let size = rd32(&mut p)?;
        let packed = rd32(&mut p)?;
        let offset = rd32(&mut p)?;
        v.push(DatEntry { path: norm(&String::from_utf8_lossy(name)), flags: comp as u32, offset: base + offset as u64, size, packed });
    }
    Ok(v)
}

fn parse_dat1(d: &[u8]) -> Result<Vec<DatEntry>> {
    let mut p = 0usize;
    let rd32 = |p: &mut usize| -> Result<u32> {
        let b = d.get(*p..*p + 4).ok_or_else(|| anyhow!("en-tête tronqué"))?;
        *p += 4;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    };
    let ndirs = rd32(&mut p)? as usize;
    let (a, b, c) = (rd32(&mut p)?, rd32(&mut p)?, rd32(&mut p)?);
    if ndirs == 0 || ndirs > 100_000 || a as usize > 1_000_000 || b > 0x100 || c > 0x7FFF_FFFF {
        bail!("en-tête invalide");
    }
    let mut names = Vec::with_capacity(ndirs);
    for _ in 0..ndirs {
        let l = *d.get(p).ok_or_else(|| anyhow!("tronqué"))? as usize;
        p += 1;
        let n = d.get(p..p + l).ok_or_else(|| anyhow!("tronqué"))?;
        p += l;
        names.push(String::from_utf8_lossy(n).to_string());
    }
    let mut v = Vec::new();
    for dn in &names {
        let nfiles = rd32(&mut p)? as usize;
        p += 12; // allocation, taille des métadonnées, horodatage
        if nfiles > 5_000_000 {
            bail!("nombre de fichiers invalide");
        }
        let dir = if dn == "." { String::new() } else { norm(dn) };
        for _ in 0..nfiles {
            let l = *d.get(p).ok_or_else(|| anyhow!("tronqué"))? as usize;
            p += 1;
            let n = d.get(p..p + l).ok_or_else(|| anyhow!("tronqué"))?;
            p += l;
            let name = String::from_utf8_lossy(n).to_string();
            let flags = rd32(&mut p)?;
            let offset = rd32(&mut p)?;
            let size = rd32(&mut p)?;
            let packed = rd32(&mut p)?;
            let path = if dir.is_empty() { norm(&name) } else { format!("{dir}/{}", norm(&name)) };
            v.push(DatEntry { path, flags, offset: offset as u64, size, packed });
        }
    }
    Ok(v)
}

/// Écrit une archive DAT2 (tests).
pub fn write_dat2(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut tree = Vec::new();
    tree.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, data, compress) in files {
        let offset = out.len() as u32;
        let packed: Vec<u8> = if *compress {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else {
            data.to_vec()
        };
        out.extend_from_slice(&packed);
        tree.extend_from_slice(&(name.len() as u32).to_le_bytes());
        tree.extend_from_slice(name.as_bytes());
        tree.push(*compress as u8);
        tree.extend_from_slice(&(data.len() as u32).to_le_bytes());
        tree.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        tree.extend_from_slice(&offset.to_le_bytes());
    }
    let tree_size = tree.len() as u32;
    out.extend_from_slice(&tree);
    let total = out.len() as u32 + 8;
    out.extend_from_slice(&tree_size.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out
}

/// Écrit une archive DAT1 avec fichiers bruts (0x20) ou LZSS par blocs (0x40) (tests).
pub fn write_dat1(dirs: &[(&str, Vec<(&str, Vec<u8>, bool)>)]) -> Vec<u8> {
    let mut idx = Vec::new();
    let be = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());
    be(&mut idx, dirs.len() as u32);
    be(&mut idx, 0x0A);
    be(&mut idx, 0);
    be(&mut idx, 0);
    for (n, _) in dirs {
        idx.push(n.len() as u8);
        idx.extend_from_slice(n.as_bytes());
    }
    // Calcul de la taille de l'index pour connaître les décalages de données.
    let mut idx_len = idx.len();
    for (_, files) in dirs {
        idx_len += 16;
        for (n, _, _) in files {
            idx_len += 1 + n.len() + 16;
        }
    }
    let mut data = Vec::new();
    let mut entries = Vec::new();
    for (_, files) in dirs {
        let mut es = Vec::new();
        for (n, content, comp) in files {
            let off = (idx_len + data.len()) as u32;
            if *comp {
                let mut packed = Vec::new();
                for chunk in content.chunks(1000) {
                    let z = lzss_literal_only(chunk);
                    packed.extend_from_slice(&(z.len() as u16).to_be_bytes());
                    packed.extend_from_slice(&z);
                }
                es.push((n.to_string(), 0x40u32, off, content.len() as u32, packed.len() as u32));
                data.extend_from_slice(&packed);
            } else {
                es.push((n.to_string(), 0x20u32, off, content.len() as u32, 0));
                data.extend_from_slice(content);
            }
        }
        entries.push(es);
    }
    for es in &entries {
        be(&mut idx, es.len() as u32);
        be(&mut idx, 0x0A);
        be(&mut idx, 0x10);
        be(&mut idx, 0);
        for (n, fl, off, size, packed) in es {
            idx.push(n.len() as u8);
            idx.extend_from_slice(n.as_bytes());
            be(&mut idx, *fl);
            be(&mut idx, *off);
            be(&mut idx, *size);
            be(&mut idx, *packed);
        }
    }
    assert_eq!(idx.len(), idx_len);
    idx.extend_from_slice(&data);
    idx
}

/// Résultat d'une extraction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Extracted {
    /// Fichiers écrits.
    pub written: usize,
    /// Fichiers déjà présents, laissés tels quels (sauf `overwrite`).
    pub skipped: usize,
}

/// Extrait un fichier, un dossier (récursivement) ou toute une archive vers `out_dir`, en conservant l'arborescence
/// relative au dossier extrait. Un fichier existant n'est remplacé que si `overwrite` est vrai.
pub fn extract(path: &Path, out_dir: &Path, overwrite: bool) -> Result<Extracted> {
    let items: Vec<(PathBuf, PathBuf)> = match stat(path) {
        Some(Stat::File) => vec![(path.to_path_buf(), PathBuf::from(path.file_name().ok_or_else(|| anyhow!("nom de fichier invalide"))?))],
        Some(Stat::Dir) => walk(path),
        None => bail!("{} : introuvable dans l'archive", path.display()),
    };
    let mut report = Extracted::default();
    for (full, rel) in items {
        // Les noms d'entrée sont déjà nettoyés (voir `norm`) ; on vérifie quand même que rien ne sort du dossier de sortie.
        if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            bail!("chemin d'entrée refusé : {}", rel.display());
        }
        let dest = out_dir.join(&rel);
        if dest.exists() && !overwrite {
            report.skipped += 1;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| anyhow!("{} : {e}", parent.display()))?;
        }
        std::fs::write(&dest, read_any(&full)?).map_err(|e| anyhow!("{} : {e}", dest.display()))?;
        report.written += 1;
    }
    Ok(report)
}
