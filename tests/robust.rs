//! Robustesse face aux fichiers corrompus ou hostiles : aucun lecteur ne doit paniquer, boucler ni allouer de façon démesurée.

use fallout_image_browser::convert::*;
use fallout_image_browser::formats::{self, DecodeCtx};
use fallout_image_browser::model::*;
use fallout_image_browser::palette;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

// Allocateur qui relève la mémoire en cours et son pic, pour repérer les allocations démesurées déclenchées par un en-tête.
struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn note(add: usize) {
    let now = CUR.fetch_add(add, Ordering::Relaxed) + add;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            note(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            note(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        CUR.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            CUR.fetch_sub(l.size(), Ordering::Relaxed);
            note(new);
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Au-delà de cette mémoire (octets) pour décoder un fichier de quelques Mo, on signale le lecteur.
const MEM_LIMIT: usize = 256 << 20;

fn ctx() -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() }
}

/// Générateur pseudo-aléatoire déterministe.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(data: &[u8], rng: &mut Rng, kind: usize) -> Vec<u8> {
    let mut d = data.to_vec();
    match kind % 5 {
        0 => d.truncate(rng.below(d.len())),
        1 => {
            for _ in 0..1 + rng.below(8) {
                let i = rng.below(d.len());
                d[i] ^= 1 << rng.below(8);
            }
        }
        2 => {
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(d.len());
                d[i] = rng.next() as u8;
            }
        }
        3 => {
            // Champ de taille poussé à une valeur extrême.
            let i = rng.below(d.len().saturating_sub(4).max(1));
            let v: u32 = [0xFFFF_FFFF, 0x7FFF_FFFF, 0x0100_0000, 0, 0xFFFF][rng.below(5)];
            for (k, b) in v.to_le_bytes().iter().enumerate() {
                if i + k < d.len() {
                    d[i + k] = *b;
                }
            }
        }
        _ => {
            let i = rng.below(d.len());
            let n = rng.below(64).min(d.len() - i);
            for b in &mut d[i..i + n] {
                *b = 0xFF;
            }
        }
    }
    d
}

fn gradient(w: usize, h: usize) -> Picture {
    let mut v = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            v[o] = (x * 255 / (w - 1)) as u8;
            v[o + 1] = (y * 255 / (h - 1)) as u8;
            v[o + 2] = ((x + y) * 5 % 256) as u8;
            if x % 7 == 0 {
                v[o + 3] = 100;
            }
        }
    }
    Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(v), None)
}

fn walk(dir: &std::path::Path, exts: &[&str], out: &mut Vec<std::path::PathBuf>, max: usize) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, exts, out, max);
            } else if out.len() < max && p.extension().map(|x| exts.iter().any(|e| x.eq_ignore_ascii_case(e))).unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

/// (format attendu, contenu) : fichiers fabriqués dans chaque format d'écriture, puis fichiers réels s'ils sont présents.
fn seeds() -> Vec<(Format, String, Vec<u8>)> {
    let pic = gradient(37, 23);
    let mut v = Vec::new();
    for f in Format::ALL {
        let caps = formats::caps(f);
        let depths: Vec<DepthChoice> = [(caps.d32, DepthChoice::Rgba32), (caps.d24, DepthChoice::Rgb24), (caps.indexed, DepthChoice::Indexed(PalChoice::Fallout))].into_iter().filter(|(ok, _)| *ok).map(|(_, d)| d).collect();
        for depth in depths {
            let o = ConvertOptions { format: f, depth, dither: false, bg: [0; 3], enc: Default::default() };
            if let Ok(b) = convert_to_bytes(&pic, &o, &Selection::default()) {
                v.push((f, format!("{} #{}", f.name(), v.len()), b));
            }
        }
    }
    // Fichiers réels versionnés avec le projet (`_test_files`) : un échantillon de chaque famille de formats.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files");
    let mut real: Vec<(Format, std::path::PathBuf)> = Vec::new();
    let mut files = Vec::new();
    walk(&root, &["frm", "rix", "art", "psd", "png", "zar", "spr", "til"], &mut files, usize::MAX);
    assert!(!files.is_empty(), "_test_files introuvable");
    for p in files {
        if std::fs::metadata(&p).map(|m| m.len() < 3_000_000).unwrap_or(false) {
            if let Some(f) = Format::from_path(&p) {
                real.push((f, p));
            }
        }
    }
    for (f, p) in real {
        if let Ok(b) = std::fs::read(&p) {
            v.push((f, p.file_name().unwrap().to_string_lossy().to_string(), b));
        }
    }
    v
}

#[test]
fn decoders_survive_corrupted_files() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut problems: Vec<String> = Vec::new();
    let mut runs = 0usize;
    let mut slowest = (Duration::ZERO, String::new());
    for (fmt, name, data) in seeds() {
        // Le fichier intact doit se lire dans son propre format.
        for k in 0..400 {
            let m = if k == 0 { data.clone() } else { mutate(&data, &mut rng, k) };
            let base = CUR.load(Ordering::Relaxed);
            PEAK.store(base, Ordering::Relaxed);
            let t = Instant::now();
            let r = catch_unwind(AssertUnwindSafe(|| formats::decode(fmt, &m, &ctx()).map(|p| p.frames.len())));
            let dt = t.elapsed();
            let used = PEAK.load(Ordering::Relaxed).saturating_sub(base);
            if used > MEM_LIMIT {
                problems.push(format!("MÉMOIRE {name} mutation {k} : {} Mo pour {} octets de fichier", used >> 20, m.len()));
            }
            runs += 1;
            if dt > slowest.0 {
                slowest = (dt, format!("{name} (mutation {k})"));
            }
            match r {
                Err(_) => problems.push(format!("PANIC  {name} mutation {k} ({} octets)", m.len())),
                Ok(Ok(_)) | Ok(Err(_)) => {}
            }
            if dt > Duration::from_secs(10) {
                problems.push(format!("LENT   {name} mutation {k} : {dt:?}"));
            }
        }
        // Fichier d'un autre format passé à chaque lecteur.
        for other in Format::ALL.iter().chain(Format::READ_ONLY.iter()) {
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = formats::decode(*other, &data, &ctx());
            }));
            runs += 1;
            if r.is_err() {
                problems.push(format!("PANIC  {name} lu comme {}", other.name()));
            }
        }
        let _ = catch_unwind(AssertUnwindSafe(|| formats::sniff(&data[..data.len().min(8)])));
    }
    println!("{runs} lectures de fichiers corrompus ; plus lente : {:?} ({})", slowest.0, slowest.1);
    assert!(problems.is_empty(), "{} problème(s) :\n{}", problems.len(), problems.join("\n"));
}

/// Courts préfixes (0 à 64 octets) de chaque seed : les en-têtes tronqués ne doivent jamais faire paniquer.
#[test]
fn decoders_survive_tiny_inputs() {
    let mut problems = Vec::new();
    for (_, name, data) in seeds() {
        for n in 0..64.min(data.len()) {
            for f in Format::ALL.iter().chain(Format::READ_ONLY.iter()) {
                if catch_unwind(AssertUnwindSafe(|| {
                    let _ = formats::decode(*f, &data[..n], &ctx());
                }))
                .is_err()
                {
                    problems.push(format!("PANIC {name}[..{n}] lu comme {}", f.name()));
                }
            }
            let _ = catch_unwind(AssertUnwindSafe(|| formats::sniff(&data[..n])));
        }
    }
    assert!(problems.is_empty(), "{} problème(s) :\n{}", problems.len(), problems.iter().take(30).cloned().collect::<Vec<_>>().join("\n"));
}
