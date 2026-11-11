//! Fenêtre modale de conversion de l'image affichée.

use crate::app::App;
use crate::batch::{self, Job};
use crate::convert::*;
use crate::i18n::{tr, trf};
use crate::model::*;
use eframe::egui;
use std::path::PathBuf;

pub use crate::convert_settings::*;
pub use crate::convert_state::*;

impl App {
    pub fn open_convert(&mut self) {
        let Some(s) = &self.shown else { return };
        self.conv.source = Some((s.loaded.clone(), Selection { dir: s.dir, frame: s.frame }));
        self.conv.name = batch::stem_of(&s.loaded.path);
        self.conv.error = None;
        self.conv.estimate = Estimate::default();
        self.conv.fix_depth();
        self.conv.open = true;
    }

    /// Conversion en un clic de l'image affichée, à côté du fichier source (même nom, extension du format) : FRM (palette Fallout),
    /// FRM avec un .PAL créé à partir de l'image (index 220 blanc pour les écrans de mort si `text_white`), ou RIX.
    pub fn quick_convert(&mut self, format: Format, with_pal: bool, text_white: bool) {
        let Some(s) = &self.shown else { return };
        let (loaded, sel) = (s.loaded.clone(), Selection { dir: s.dir, frame: s.frame });
        let Some(out_dir) = crate::dat::real_parent(&loaded.path) else { return };
        let settings = ConvSettings {
            fmt: format.ext().to_string(),
            depth: match (format, with_pal) {
                (Format::Frm, true) => DepthKind::Custom8,
                (Format::Frm, false) => DepthKind::Fallout8,
                _ => DepthKind::Pal8,
            },
            create_pal: true,
            text_white,
            ..ConvSettings::default()
        };
        let Some(opts) = ConvertUi::new(&settings).options() else { return };
        let job = Job { opts, seq: false, policy: self.policy.clone() };
        match batch::convert_loaded(&loaded, &sel, &job, &out_dir, None) {
            Ok(files) => {
                let what = files.first().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.toast(trf("c.done", &[("file", what)]));
                self.refresh_if_current(Some(out_dir));
            }
            Err(e) => self.toast(e),
        }
    }

    fn start_conversion(&mut self, ctx: &egui::Context) {
        let Some(opts) = self.conv.options() else {
            self.conv.error = Some(tr("c.need_pal"));
            return;
        };
        let (Some((loaded, sel)), Some(out_dir)) = (self.conv.source.clone(), self.conv.effective_out_dir()) else { return };
        let job = Job { opts, seq: self.conv.s.sequence, policy: self.policy.clone() };
        let name = self.conv.name.clone();
        self.conv.error = None;
        self.conv.running = true;
        *self.conv.outcome.lock().unwrap() = None;
        let outcome = self.conv.outcome.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| batch::convert_loaded(&loaded, &sel, &job, &out_dir, Some(&name))))
                .unwrap_or_else(|_| Err("erreur interne".into()));
            *outcome.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
    }

    fn poll_conversion(&mut self) {
        if !self.conv.running {
            return;
        }
        let done = self.conv.outcome.lock().unwrap().take();
        let Some(res) = done else { return };
        self.conv.running = false;
        match res {
            Ok(files) => {
                let out_dir = self.conv.effective_out_dir();
                self.conv.s.out_dir = out_dir.clone();
                self.conv.s.last_out_dir = out_dir.clone();
                let what = match files.len() {
                    1 => files[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    n => format!("{n}"),
                };
                self.conv.open = false;
                self.toast(trf("c.done", &[("file", what)]));
                self.refresh_if_current(out_dir);
            }
            Err(e) => self.conv.error = Some(e),
        }
    }

    /// Rafraîchit la liste des fichiers si l'on vient d'écrire dans le dossier courant.
    pub fn refresh_if_current(&mut self, out_dir: Option<PathBuf>) {
        if let (Some(dir), Some(cur)) = (self.tree.selected_dir.clone(), out_dir)
            && dir == cur
        {
            self.tree.reload_dir(&dir);
            self.files = self.tree.files_of(&dir);
            if let Some(w) = &self.wanted {
                self.index = self.files.iter().position(|f| f == w);
            }
        }
    }

    pub fn convert_window(&mut self, ctx: &egui::Context) {
        self.poll_conversion();
        if !self.conv.open {
            return;
        }
        self.conv.update_estimate(ctx);
        let mut start = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("convert_modal")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(500.0);
            crate::dialog::setup(ui);
            if crate::dialog::title(ui, &tr("c.title")) && !self.conv.running {
                close = true;
            }
            if let Some((l, _)) = &self.conv.source {
                ui.label(egui::RichText::new(l.path.display().to_string()).weak());
            }
            ui.add_space(6.0);
            let running = self.conv.running;
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("conv_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    let multi = self.conv.source.as_ref().is_some_and(|(l, _)| l.pic.frames.len() > 1);
                    self.conv.options_rows(ui, multi);
                    self.conv.estimate_row(ui);
                    ui.label(tr("c.name"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.conv.name).desired_width(240.0));
                        ui.label(format!(".{}", self.conv.format().ext()));
                    });
                    ui.end_row();
                    ui.label(tr("c.out_dir"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            if ui.button(tr("c.choose")).clicked() {
                                let mut d = rfd::FileDialog::new();
                                if let Some(cur) = self.conv.effective_out_dir() {
                                    d = d.set_directory(cur);
                                }
                                if let Some(p) = d.pick_folder() {
                                    self.conv.s.out_dir = Some(p);
                                }
                            }
                            if ui.button(tr("c.out_source")).clicked() {
                                self.conv.s.out_dir = self.conv.source_dir();
                            }
                            let last = self.conv.s.last_out_dir.clone();
                            if ui.add_enabled(last.is_some(), egui::Button::new(tr("c.out_last"))).clicked() {
                                self.conv.s.out_dir = last;
                            }
                        });
                        if let Some(p) = self.conv.effective_out_dir() {
                            ui.label(egui::RichText::new(p.display().to_string()));
                        }
                    });
                    ui.end_row();
                });
            });
            if let Some(e) = &self.conv.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let can = !self.conv.running && self.conv.effective_out_dir().is_some() && !self.conv.name.trim().is_empty();
                if crate::dialog::button_enabled(ui, can, tr("c.start")).clicked() {
                    start = true;
                }
                if crate::dialog::button_enabled(ui, !self.conv.running, tr("c.cancel")).clicked() {
                    close = true;
                }
                if self.conv.running {
                    ui.spinner();
                }
            });
        });
        if modal.should_close() && !self.conv.running {
            close = true;
        }
        if close {
            self.conv.open = false;
        }
        if start {
            self.start_conversion(ctx);
        }
    }
}
