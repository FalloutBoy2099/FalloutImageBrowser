//! Miniatures du mode dossier : décodage en arrière-plan (seules les miniatures visibles sont demandées), cache de textures.

use crate::loader::{PalPolicy, load_file};
use crate::model::frame_to_rgba;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};

/// Une miniature : fichier et taille de la boîte, en pixels écran.
pub type Key = (PathBuf, u32);

/// Fichiers plus gros que cela (sur le disque) : pas de miniature, pour ne pas bloquer un thread de décodage.
const MAX_FILE: u64 = 96 << 20;

/// Nombre de textures conservées ; au-delà, les moins récemment affichées sont libérées.
const MAX_CACHED: usize = 1500;

pub enum Thumb {
    /// Texture et sa taille en pixels.
    Ready(TextureHandle, [usize; 2]),
    Failed,
}

struct Entry {
    thumb: Thumb,
    used: u64,
}

struct Queue {
    jobs: Mutex<Vec<(Key, PalPolicy)>>,
    cv: Condvar,
}

struct Out {
    key: Key,
    image: Option<ColorImage>,
}

pub struct Thumbs {
    q: Arc<Queue>,
    rx: Receiver<Out>,
    cache: HashMap<Key, Entry>,
    in_flight: HashSet<Key>,
    tick: u64,
}

/// Image réduite pour tenir dans une boîte de `px` pixels (plus grand côté) ; une image plus petite reste à sa taille.
fn make(key: &Key, policy: &PalPolicy) -> Option<ColorImage> {
    let (path, px) = (&key.0, key.1 as usize);
    if !crate::dat::is_virtual(path) && std::fs::metadata(path).map(|m| m.len() > MAX_FILE).unwrap_or(false) {
        return None;
    }
    let loaded = load_file(path, policy).ok()?;
    let pic = &loaded.pic;
    let f = pic.frames.first()?;
    let (w, h) = (f.width as usize, f.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = frame_to_rgba(f, pic.palette.as_ref());
    if w.max(h) <= px {
        return Some(ColorImage::from_rgba_unmultiplied([w, h], &rgba));
    }
    let scale = px as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba)?;
    let small = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    Some(ColorImage::from_rgba_unmultiplied([nw as usize, nh as usize], small.as_raw()))
}

impl Thumbs {
    pub fn new(ctx: egui::Context) -> Thumbs {
        let q = Arc::new(Queue { jobs: Mutex::new(Vec::new()), cv: Condvar::new() });
        let (tx, rx) = channel::<Out>();
        let n = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) / 2).clamp(1, 3);
        for _ in 0..n {
            let (q, tx, ctx) = (q.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                loop {
                    let (key, policy) = {
                        let mut g = q.jobs.lock().unwrap();
                        loop {
                            if !g.is_empty() {
                                break g.remove(0);
                            }
                            g = q.cv.wait(g).unwrap();
                        }
                    };
                    let image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(&key, &policy))).ok().flatten();
                    if tx.send(Out { key, image }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Thumbs { q, rx, cache: HashMap::new(), in_flight: HashSet::new(), tick: 0 }
    }

    /// Reçoit les miniatures terminées.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(out) = self.rx.try_recv() {
            self.in_flight.remove(&out.key);
            let thumb = match out.image {
                Some(img) => {
                    let size = img.size;
                    Thumb::Ready(ctx.load_texture(format!("thumb:{}:{}", out.key.0.display(), out.key.1), img, TextureOptions::NEAREST), size)
                }
                None => Thumb::Failed,
            };
            self.cache.insert(out.key, Entry { thumb, used: self.tick });
        }
        if self.cache.len() > MAX_CACHED {
            let mut by_age: Vec<(u64, Key)> = self.cache.iter().map(|(k, e)| (e.used, k.clone())).collect();
            by_age.sort_by_key(|(u, _)| *u);
            for (_, k) in by_age.into_iter().take(self.cache.len() - MAX_CACHED * 3 / 4) {
                self.cache.remove(&k);
            }
        }
    }

    /// Début d'une image d'affichage : sert à retenir ce qui est visible (pour l'éviction du cache).
    pub fn begin_frame(&mut self) {
        self.tick += 1;
    }

    /// Miniature prête (ou échec) pour cette clé.
    pub fn get(&mut self, key: &Key) -> Option<&Thumb> {
        let tick = self.tick;
        self.cache.get_mut(key).map(|e| {
            e.used = tick;
            &e.thumb
        })
    }

    /// Les miniatures visibles qu'il reste à produire, dans l'ordre : remplace la file d'attente (ce qui n'est plus visible est abandonné).
    pub fn want(&mut self, visible: Vec<Key>, policy: &PalPolicy) {
        let mut g = self.q.jobs.lock().unwrap();
        for (k, _) in g.drain(..) {
            self.in_flight.remove(&k);
        }
        for k in visible {
            if !self.cache.contains_key(&k) && !self.in_flight.contains(&k) {
                self.in_flight.insert(k.clone());
                g.push((k, policy.clone()));
            }
        }
        if !g.is_empty() {
            self.q.cv.notify_all();
        }
    }

    pub fn is_pending(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Oublie toutes les miniatures (palette changée, dossier relu).
    pub fn clear(&mut self) {
        self.cache.clear();
    }
}
