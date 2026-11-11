//! Ouverture de fichiers et de dossiers, navigation entre les images.

use crate::app::{App, PendingOpen};
use crate::cache::{PREFETCH_MAX_BYTES, Shown};
use crate::i18n;
use crate::loader::Loaded;
use crate::model::*;
use eframe::egui::{self, Vec2};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

impl App {
    /// Ouverture depuis l'extérieur (ligne de commande, glisser-déposer, boîte de dialogue) :
    /// un fichier ouvre son dossier, qui devient le dossier courant ; une archive .DAT s'ouvre comme un dossier.
    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        // Archive pas encore analysée : l'analyse tourne en arrière-plan et l'ouverture reprend à sa fin (voir `poll_archives`).
        if crate::dat::archive_pending(path) {
            self.pending_open = Some((path.to_path_buf(), PendingOpen::Path));
            return;
        }
        let kind = match crate::dat::stat_nowait(path) {
            Some(crate::dat::Stat::File) => Some(false),
            Some(crate::dat::Stat::Dir) => Some(true),
            None if path.is_dir() => Some(true),
            None if path.is_file() => Some(false),
            None => None,
        };
        match kind {
            Some(true) => self.open_dir(ctx, path, true),
            Some(false) => {
                if let Some(parent) = path.parent() {
                    self.open_dir(ctx, parent, false);
                }
                if crate::model::is_supported(path) {
                    self.show_file(ctx, path);
                }
            }
            None => return,
        }
        self.reveal_in_tree();
    }

    pub fn open_dir(&mut self, ctx: &egui::Context, dir: &Path, open_first: bool) {
        // Archive pas encore analysée : le dossier est sélectionné tout de suite, la liste des fichiers suit à la fin de l'analyse.
        if crate::dat::archive_pending(dir) {
            self.pending_open = Some((dir.to_path_buf(), PendingOpen::Dir { open_first }));
        }
        self.tree.reveal_dir(dir);
        self.settings.last_dir = Some(dir.to_path_buf());
        self.files = self.tree.files_of(dir);
        self.index = None;
        if open_first {
            if let Some(first) = self.files.first().cloned() {
                self.show_file(ctx, &first);
            } else {
                self.wanted = None;
                self.shown = None;
                self.error = None;
            }
        } else if let Some(w) = &self.wanted {
            self.index = self.files.iter().position(|f| f == w);
        }
    }

    /// Applique le tri et le filtre choisis : la liste des fichiers (et donc précédent / suivant) suit l'arborescence.
    pub fn apply_view_options(&mut self) {
        self.tree.sort = self.settings.sort;
        self.tree.descending = self.settings.sort_desc;
        self.tree.filter = self.file_filter.clone();
        self.tree.rebuild();
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.files = self.tree.files_of(&dir);
            self.index = self.wanted.as_ref().and_then(|w| self.files.iter().position(|f| f == w));
        }
    }

    pub fn reload_dir(&mut self, ctx: &egui::Context) {
        crate::dat::clear_failed();
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.tree.reload_dir(&dir);
            self.files = self.tree.files_of(&dir);
            if let Some(w) = self.wanted.clone() {
                self.index = self.files.iter().position(|f| *f == w);
            }
            self.cache.clear();
            self.thumbs.clear();
            if let Some(w) = self.wanted.clone() {
                self.show_file(ctx, &w);
            }
        }
    }

    /// Recharge un dossier de l'arborescence depuis le disque (le dossier courant : liste des fichiers et image comprises).
    pub fn refresh_dir(&mut self, ctx: &egui::Context, dir: &Path) {
        if self.tree.selected_dir.as_deref() == Some(dir) {
            self.reload_dir(ctx);
        } else {
            self.tree.reload_dir(dir);
        }
    }

    /// Crée une animation FRM / ART à partir des images du dossier et l'affiche.
    pub fn create_animation(&mut self, ctx: &egui::Context, dir: &Path, out_dir: &Path, name: &str, format: Format) {
        match crate::batch::build_animation(dir, out_dir, name, format, &self.policy) {
            Ok((out, n)) => {
                self.tree.reload_dir(out_dir);
                self.open_dir(ctx, out_dir, false);
                self.show_file(ctx, &out);
                self.reveal_in_tree();
                let file = out.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                self.toast(i18n::trf("t.anim_created", &[("file", file), ("n", n.to_string())]));
            }
            Err(e) => self.toast(e),
        }
    }

    /// Fait défiler l'arborescence jusqu'à l'élément courant (une seule fois, sur demande explicite).
    pub fn reveal_in_tree(&mut self) {
        let target = self.wanted.clone().or_else(|| self.tree.selected_dir.clone());
        if let Some(t) = target {
            self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == t);
        }
    }

    pub fn show_file(&mut self, ctx: &egui::Context, path: &Path) {
        self.wanted = Some(path.to_path_buf());
        self.index = self.files.iter().position(|f| f == path);
        self.error = None;
        if let Some(l) = self.cache.get(path) {
            self.set_shown(ctx, l);
        } else {
            self.loader.request(path.to_path_buf(), self.policy.clone());
        }
    }

    pub fn step(&mut self, ctx: &egui::Context, delta: i32) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as i32;
        let cur = self.index.map(|i| i as i32).unwrap_or(if delta > 0 { -1 } else { 0 });
        let next = (cur + delta).rem_euclid(n) as usize;
        let p = self.files[next].clone();
        self.show_file(ctx, &p);
    }

    pub fn goto(&mut self, ctx: &egui::Context, idx: usize) {
        if let Some(p) = self.files.get(idx).cloned() {
            self.show_file(ctx, &p);
        }
    }

    pub fn set_shown(&mut self, ctx: &egui::Context, l: Arc<Loaded>) {
        let animated = l.pic.is_animated();
        self.shown = Some(Shown {
            loaded: l.clone(),
            dir: 0,
            frame: 0,
            tex: HashMap::new(),
            playing: animated && self.settings.autoplay,
            clock: 0.0,
            cycle_ok: crate::cycling::uses_cycling(&l.pic),
            cycle_phase: 0,
        });
        // Origine fixe : la vue garde sa place, le canevas replace l'ancrage de la nouvelle image au même endroit.
        let keep_view = self.settings.lock_origin && !self.fit && matches!(l.pic.format, Format::Frm | Format::Art | Format::Spr);
        if !keep_view {
            self.pan = Vec2::ZERO;
        }
        self.error = None;
        let name = l.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let title = format!("{name} — Fallout Image Browser");
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.prefetch_neighbours();
    }

    fn prefetch_neighbours(&mut self) {
        let Some(i) = self.index else { return };
        let n = self.files.len();
        if n < 2 {
            return;
        }
        let mut v = Vec::new();
        for d in [1i32, -1, 2, -2] {
            let j = (i as i32 + d).rem_euclid(n as i32) as usize;
            let p = self.files[j].clone();
            // Les très gros fichiers ne sont pas préchargés : leur décodage coûte beaucoup de mémoire pour une image peut-être jamais vue.
            let huge = std::fs::metadata(&p).is_ok_and(|m| m.len() > PREFETCH_MAX_BYTES);
            if !huge && !self.cache.contains(&p) && !v.contains(&p) && self.files[j] != self.files[i] {
                v.push(p);
            }
        }
        if !v.is_empty() {
            self.loader.prefetch(v, &self.policy);
        }
    }
}
