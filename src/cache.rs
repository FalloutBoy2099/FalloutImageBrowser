//! Image affichée, textures et cache des images déjà décodées.

use crate::loader::Loaded;
use crate::model::*;
use eframe::egui::TextureHandle;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Taille maximale d'un fichier (sur le disque) préchargé avec les images voisines.
/// Favoris ajustables (les trois autres boutons sont réservés : Bureau, Documents, Téléchargements).
pub const MAX_FAVORITES: usize = 5;

pub(crate) const PREFETCH_MAX_BYTES: u64 = 24 << 20;

pub struct TexEntry {
    pub handle: TextureHandle,
    /// Taille en mémoire graphique (RGBA 8 bits).
    pub bytes: usize,
    /// Dernier passage d'affichage qui l'a demandée (éviction de la moins récente).
    pub used: u64,
}

pub struct Shown {
    pub loaded: Arc<Loaded>,
    pub dir: usize,
    pub frame: usize,
    /// Textures : (direction, image, niveau de réduction, transformation de la vue).
    pub tex: HashMap<(usize, usize, u8, u8), TexEntry>,
    pub playing: bool,
    pub clock: f32,
    /// L'image utilise des couleurs animées de la palette Fallout (slime, feux, écrans...).
    pub cycle_ok: bool,
    /// Cran de l'animation de palette dessiné par les textures actuelles.
    pub cycle_phase: u64,
}

#[derive(Default)]
pub struct Cache {
    map: HashMap<PathBuf, Arc<Loaded>>,
    order: VecDeque<PathBuf>,
    bytes: usize,
}

fn est_bytes(l: &Loaded) -> usize {
    l.pic
        .frames
        .iter()
        .map(|f| match &f.pixels {
            Pixels::Indexed(v) => v.len(),
            Pixels::Rgba(v) => v.len(),
        })
        .sum()
}

impl Cache {
    pub fn get(&mut self, p: &Path) -> Option<Arc<Loaded>> {
        let v = self.map.get(p).cloned()?;
        self.order.retain(|x| x != p);
        self.order.push_back(p.to_path_buf());
        Some(v)
    }
    pub fn insert(&mut self, l: Arc<Loaded>) {
        let p = l.path.clone();
        if let Some(old) = self.map.insert(p.clone(), l.clone()) {
            self.bytes = self.bytes.saturating_sub(est_bytes(&old));
        }
        self.bytes += est_bytes(&l);
        self.order.retain(|x| *x != p);
        self.order.push_back(p);
        while (self.order.len() > 12 || self.bytes > 700_000_000) && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front()
                && let Some(v) = self.map.remove(&old)
            {
                self.bytes = self.bytes.saturating_sub(est_bytes(&v));
            }
        }
    }
    pub fn contains(&self, p: &Path) -> bool {
        self.map.contains_key(p)
    }
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }
}

