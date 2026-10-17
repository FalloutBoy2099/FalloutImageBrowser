//! Actions de l'application : zoom, plein écran, diaporama, ouverture, extraction, copie.

use crate::app::App;
use crate::i18n::{self, tr};
use crate::model::*;
use eframe::egui::{self, ColorImage};
use std::time::Duration;

use super::{MAX_ZOOM, MIN_ZOOM};

impl App {
    pub fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    pub fn toggle_slideshow(&mut self, ctx: &egui::Context) {
        self.slideshow = !self.slideshow;
        self.slideshow_t = ctx.input(|i| i.time);
    }

    /// Diaporama : image suivante à intervalle régulier, en revenant à la première après la dernière.
    pub fn tick_slideshow(&mut self, ctx: &egui::Context) {
        if !self.slideshow {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.slideshow_t >= self.settings.slideshow_secs.max(1) as f64 {
            self.slideshow_t = now;
            match self.index {
                Some(i) if i + 1 >= self.files.len() => self.goto(ctx, 0),
                _ => self.step(ctx, 1),
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Associe des extensions de fichier à l'application (menu Aide) et affiche le résultat.
    pub fn associate_files(&mut self, types: &[crate::assoc::FileType]) {
        let exts = crate::assoc::list(types);
        match crate::assoc::associate(types) {
            Ok(()) => {
                let key = if cfg!(windows) { "assoc.done_win" } else { "assoc.done" };
                self.toast(i18n::trf(key, &[("exts", exts)]));
            }
            Err(e) => self.toast(i18n::trf("assoc.failed", &[("exts", exts), ("error", e)])),
        }
    }

    /// Copie l'image affichée (image courante d'une animation) dans le presse-papiers du système.
    pub fn copy_image(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.shown else { return };
        let pic = &s.loaded.pic;
        let f = &pic.frames[pic.frame_index(s.dir, s.frame)];
        let rgba = frame_to_rgba(f, pic.palette.as_ref());
        let image = ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &rgba);
        ctx.copy_image(image);
        self.toast(tr("t.image_copied"));
    }

    pub fn open_in_default_program(&mut self) {
        if let Some(p) = self.shown.as_ref().map(|s| s.loaded.path.clone())
            && let Err(e) = crate::opener::open_default(&p)
        {
            self.toast(e);
        }
    }

    pub fn zoom_by(&mut self, f: f32) {
        self.fit = false;
        self.zoom = (self.zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= f;
    }

    pub fn zoom_actual(&mut self) {
        self.fit = false;
        let r = 1.0 / self.zoom.max(1e-6);
        self.zoom = 1.0;
        self.pan *= r;
    }

    /// Demande un dossier de sortie puis extrait, dans un thread, le fichier ou dossier d'archive `.dat` choisi.
    pub fn extract_from_archive(&mut self, path: &std::path::Path) {
        if self.extract_job.is_some() {
            return;
        }
        let mut d = rfd::FileDialog::new().set_title(tr("t.extract"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(crate::dat::real_parent(dir).unwrap_or_else(|| dir.clone()));
        }
        let Some(out) = d.pick_folder() else { return };
        let (src, dest) = (path.to_path_buf(), out.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::dat::extract(&src, &dest, false).map_err(|e| e.to_string()));
        });
        self.extract_job = Some((rx, out));
        self.toast(tr("t.extracting"));
    }

    /// Récupère le résultat d'une extraction terminée et l'affiche.
    pub fn poll_extract(&mut self, ctx: &egui::Context) {
        let Some((rx, out)) = &self.extract_job else { return };
        match rx.try_recv() {
            Ok(result) => {
                let dir = out.display().to_string();
                self.extract_job = None;
                match result {
                    Ok(r) => self.toast(i18n::trf("t.extract_done", &[("written", r.written.to_string()), ("skipped", r.skipped.to_string()), ("dir", dir)])),
                    Err(e) => self.toast(i18n::trf("t.extract_failed", &[("error", e)])),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.extract_job = None,
        }
    }

    pub fn pick_folder(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_folder"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.open_dir(ctx, &p, true);
        }
    }

    pub fn pick_file(&mut self, ctx: &egui::Context) {
        let exts: Vec<&str> = vec![
            "art", "bmp", "frm", "fr0", "fr1", "fr2", "fr3", "fr4", "fr5", "gif", "jpg", "jpeg", "pcx", "rix", "tif", "tiff", "tga", "webp", "png", "psd", "psb", "pdn", "xcf",
            "kra", "ora", "dds", "qoi", "pbm", "pgm", "ppm", "pnm", "pam", "ase", "aseprite", "pal", "act", "gpl", "hex", "kpl", "zar", "til", "spr",
        ];
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_file")).add_filter(tr("m.images"), &exts);
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p);
        }
    }

    pub fn pick_dat(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_dat")).add_filter("DAT / ZIP / 7Z", &["dat", "DAT", "zip", "ZIP", "7z", "7Z"]);
        if let Some(dir) = &self.settings.last_dir
            && let Some(p) = crate::dat::real_parent(dir)
        {
            d = d.set_directory(p);
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p);
        }
    }

    /// Demande un dossier puis copie ou déplace le fichier affiché ; après un déplacement, le fichier suivant s'affiche.
    pub fn copy_or_move_to(&mut self, ctx: &egui::Context, path: &std::path::Path, mv: bool) {
        let mut d = rfd::FileDialog::new().set_title(tr(if mv { "t.move_to" } else { "t.copy_to" }));
        if let Some(dir) = crate::dat::real_parent(path) {
            d = d.set_directory(dir);
        }
        let Some(dest) = d.pick_folder() else { return };
        let result = if mv { crate::fileops::move_to(path, &dest) } else { crate::fileops::copy_to(path, &dest) };
        match result {
            Ok(new) => {
                if mv {
                    let idx = self.index.unwrap_or(0);
                    // Rien à réafficher de force : le fichier déplacé n'est plus dans le dossier.
                    self.wanted = None;
                    self.reload_dir(ctx);
                    match self.files.get(idx.min(self.files.len().saturating_sub(1))).cloned() {
                        Some(next) => self.show_file(ctx, &next),
                        None => {
                            self.shown = None;
                            self.error = None;
                        }
                    }
                }
                self.toast(i18n::trf(if mv { "t.moved_to" } else { "t.copied_to" }, &[("path", new.display().to_string())]));
            }
            Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
        }
    }
}
