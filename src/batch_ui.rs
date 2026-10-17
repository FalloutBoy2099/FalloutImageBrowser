//! Fenêtre indépendante de conversion par lot : liste de fichiers, dossiers ou archives, mêmes réglages que la conversion d'une
//! image, dossier de sortie, barre de progression et bilan. Elle ne bloque pas la fenêtre principale.

use crate::app::App;
use crate::batch::{self, BatchItem, BatchOut, Job, Progress};
use crate::convert::Selection;
use crate::convert_ui::{ConvSettings, ConvertUi};
use crate::i18n::{tr, trf};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Réglages mémorisés d'un lot à l'autre.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct BatchSettings {
    pub conv: ConvSettings,
    /// Ajouter aussi les sous-dossiers d'un dossier choisi.
    pub recursive: bool,
    /// Écrire dans un seul dossier (sinon : à côté de chaque fichier source).
    pub single_folder: bool,
    /// Recréer les sous-dossiers des dossiers ajoutés sous le dossier de sortie.
    pub keep_tree: bool,
}

impl Default for BatchSettings {
    fn default() -> Self {
        BatchSettings { conv: ConvSettings::default(), recursive: true, single_folder: false, keep_tree: true }
    }
}

pub struct BatchUi {
    pub open: bool,
    /// Réglages de conversion (sans image source) : même objet que la fenêtre de conversion d'une image.
    pub conv: ConvertUi,
    pub recursive: bool,
    pub single_folder: bool,
    pub keep_tree: bool,
    pub items: Vec<BatchItem>,
    /// Lot en cours ou terminé (pour le bilan).
    pub progress: Option<Arc<Progress>>,
    /// Sortie du dernier lot, pour rafraîchir la liste des fichiers à la fin.
    out: Option<BatchOut>,
    reported: bool,
}

impl BatchUi {
    pub fn new(s: &BatchSettings) -> BatchUi {
        BatchUi {
            open: false,
            conv: ConvertUi::new(&s.conv),
            recursive: s.recursive,
            single_folder: s.single_folder,
            keep_tree: s.keep_tree,
            items: Vec::new(),
            progress: None,
            out: None,
            reported: true,
        }
    }

    pub fn to_settings(&self) -> BatchSettings {
        BatchSettings { conv: self.conv.to_settings(), recursive: self.recursive, single_folder: self.single_folder, keep_tree: self.keep_tree }
    }

    fn running(&self) -> bool {
        self.progress.as_ref().is_some_and(|p| !p.finished.load(Ordering::Acquire))
    }

    /// Ajoute des éléments sans doublon.
    fn add(&mut self, new: Vec<BatchItem>) {
        let have: std::collections::HashSet<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        self.items.extend(new.into_iter().filter(|i| !have.contains(&i.path)));
    }

    fn out_dir(&self) -> Option<PathBuf> {
        self.conv.s.out_dir.clone().or_else(|| self.conv.s.last_out_dir.clone())
    }

    fn batch_out(&self) -> Option<BatchOut> {
        if self.single_folder { Some(BatchOut::Folder { dir: self.out_dir()?, keep_tree: self.keep_tree }) } else { Some(BatchOut::NextToSource) }
    }
}

impl App {
    pub fn open_batch(&mut self) {
        self.batch.conv.error = None;
        self.batch.conv.fix_depth();
        self.batch.open = true;
    }

    fn batch_add_path(&mut self, path: &std::path::Path) {
        match batch::list_inputs(path, self.batch.recursive) {
            Ok((v, _, _)) => {
                let items = v.into_iter().map(|(path, rel)| BatchItem { path, rel }).collect();
                self.batch.add(items);
            }
            Err(e) => self.batch.conv.error = Some(e),
        }
    }

    fn start_batch(&mut self, ctx: &egui::Context) {
        let Some(opts) = self.batch.conv.options() else {
            self.batch.conv.error = Some(tr("c.need_pal"));
            return;
        };
        let Some(out) = self.batch.batch_out() else { return };
        if self.batch.items.is_empty() {
            return;
        }
        let job = Job { opts, seq: self.batch.conv.s.sequence, policy: self.policy.clone() };
        let items = self.batch.items.clone();
        let progress = Arc::new(Progress::new(items.len()));
        self.batch.progress = Some(progress.clone());
        self.batch.out = Some(out.clone());
        self.batch.reported = false;
        self.batch.conv.error = None;
        if let BatchOut::Folder { dir, .. } = &out {
            self.batch.conv.s.last_out_dir = Some(dir.clone());
        }
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Le rapport d'avancement réveille l'interface régulièrement : elle dessine la barre de progression.
            let waker = {
                let (progress, ctx) = (progress.clone(), ctx.clone());
                std::thread::spawn(move || {
                    while !progress.finished.load(Ordering::Acquire) {
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        ctx.request_repaint();
                    }
                })
            };
            batch::run_batch(&items, &job, &out, &Selection::default(), &progress);
            let _ = waker.join();
            ctx.request_repaint();
        });
    }

    /// Bilan à la fin d'un lot : message et rafraîchissement de la liste des fichiers du dossier courant.
    fn finish_batch_report(&mut self) {
        let Some(p) = self.batch.progress.clone() else { return };
        if self.batch.reported || !p.finished.load(Ordering::Acquire) {
            return;
        }
        self.batch.reported = true;
        let (done, failed) = (p.done.load(Ordering::Relaxed), p.failed.load(Ordering::Relaxed));
        self.toast(trf("b.summary", &[("ok", (done - failed).to_string()), ("failed", failed.to_string())]));
        let dir = match &self.batch.out {
            Some(BatchOut::Folder { dir, .. }) => Some(dir.clone()),
            _ => self.tree.selected_dir.clone(),
        };
        self.refresh_if_current(dir);
    }

    pub fn batch_window(&mut self, ctx: &egui::Context) {
        self.finish_batch_report();
        if !self.batch.open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let running = self.batch.running();
        let native = self.settings.taskbar_windows;
        crate::ui::extra_window(ctx, native, "batch_window", tr("b.title"), [600.0, 560.0], true, false, &mut open, |ui| {
            crate::dialog::setup(ui);
            ui.add_enabled_ui(!running, |ui| {
                // --- Fichiers à convertir
                ui.label(egui::RichText::new(tr("b.sources")).strong());
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("b.add_files")).clicked()
                        && let Some(files) = rfd::FileDialog::new().pick_files()
                    {
                        let items = files.into_iter().filter(|p| batch::convertible(p)).map(|path| BatchItem { path, rel: PathBuf::new() }).collect();
                        self.batch.add(items);
                    }
                    if ui.button(tr("b.add_folder")).clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        self.batch_add_path(&dir);
                    }
                    if ui.button(tr("b.add_dat")).clicked()
                        && let Some(dat) = rfd::FileDialog::new().add_filter("DAT / ZIP / 7Z", &["dat", "DAT", "zip", "ZIP", "7z", "7Z"]).pick_file()
                    {
                        self.batch_add_path(&dat);
                    }
                    let has_files = !self.files.is_empty();
                    if ui.add_enabled(has_files, egui::Button::new(tr("b.add_current"))).clicked() {
                        let items = self.files.iter().filter(|p| batch::convertible(p)).map(|p| BatchItem { path: p.clone(), rel: PathBuf::new() }).collect();
                        self.batch.add(items);
                    }
                });
                ui.checkbox(&mut self.batch.recursive, tr("b.recursive"));
                let n = self.batch.items.len();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(trf("b.count", &[("n", n.to_string())])).weak());
                    if ui.add_enabled(n > 0, egui::Button::new(tr("b.clear"))).clicked() {
                        self.batch.items.clear();
                    }
                });
                let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(row_h * 7.0).auto_shrink([false, true]).show_rows(ui, row_h, n.max(1), |ui, range| {
                        if n == 0 {
                            ui.label(egui::RichText::new(tr("b.empty")).weak());
                        }
                        for i in range.filter(|i| *i < n) {
                            let it = &self.batch.items[i];
                            ui.add(egui::Label::new(it.path.display().to_string()).truncate());
                        }
                    });
                });
                ui.separator();
                // --- Réglages de conversion
                egui::Grid::new("batch_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    self.batch.conv.options_rows(ui, true);
                    ui.label(tr("c.out_dir"));
                    ui.vertical(|ui| {
                        ui.radio_value(&mut self.batch.single_folder, false, tr("b.out_source"));
                        ui.radio_value(&mut self.batch.single_folder, true, tr("b.out_folder"));
                        ui.add_enabled_ui(self.batch.single_folder, |ui| {
                            ui.horizontal(|ui| {
                                if ui.button(tr("c.choose")).clicked() {
                                    let mut d = rfd::FileDialog::new();
                                    if let Some(cur) = self.batch.out_dir() {
                                        d = d.set_directory(cur);
                                    }
                                    if let Some(p) = d.pick_folder() {
                                        self.batch.conv.s.out_dir = Some(p);
                                    }
                                }
                                if let Some(p) = self.batch.out_dir() {
                                    ui.label(p.display().to_string());
                                }
                            });
                            ui.checkbox(&mut self.batch.keep_tree, tr("b.keep_tree"));
                        });
                    });
                    ui.end_row();
                });
            });
            if let Some(e) = &self.batch.conv.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            // --- Avancement et bilan
            if let Some(p) = self.batch.progress.clone() {
                let (done, total) = (p.done.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed).max(1));
                let failed = p.failed.load(Ordering::Relaxed);
                ui.add_space(4.0);
                ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done} / {total}")).desired_height(22.0));
                if p.finished.load(Ordering::Acquire) {
                    let ok = done - failed;
                    let text = trf("b.summary", &[("ok", ok.to_string()), ("failed", failed.to_string())]);
                    ui.label(if p.is_cancelled() { format!("{text}  ({})", tr("b.stopped")) } else { text });
                    let errors = p.errors.lock().map(|e| e.clone()).unwrap_or_default();
                    if !errors.is_empty() {
                        egui::CollapsingHeader::new(tr("b.errors")).default_open(true).show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                for e in &errors {
                                    ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
                                }
                            });
                        });
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                let can = !running && !self.batch.items.is_empty() && self.batch.batch_out().is_some();
                if crate::dialog::button_enabled(ui, can, tr("c.start")).clicked() {
                    start = true;
                }
                if running {
                    if crate::dialog::button(ui, tr("b.stop")).clicked()
                        && let Some(p) = &self.batch.progress
                    {
                        p.cancel();
                    }
                    ui.spinner();
                }
            });
        });
        if !open && !running {
            self.batch.open = false;
        }
        if start {
            self.start_batch(ctx);
        }
    }
}
