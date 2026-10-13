//! Éditeur de l'en-tête d'un FRM : vitesse, image d'action et décalages de chaque direction, sans toucher aux images.

use crate::app::App;
use crate::formats::frm::{self, FrmHeader};
use crate::i18n::{tr, trf};
use crate::model::Format;
use eframe::egui;
use std::path::PathBuf;

/// État de la fenêtre : le fichier d'origine, son en-tête et les valeurs en cours d'édition.
pub struct FrmDlg {
    path: PathBuf,
    data: Vec<u8>,
    original: FrmHeader,
    fps: u16,
    action: u16,
    shift_x: [i16; 6],
    shift_y: [i16; 6],
    error: Option<String>,
}

impl FrmDlg {
    fn changed(&self) -> bool {
        (self.fps, self.action, self.shift_x, self.shift_y) != (self.original.fps, self.original.action_frame, self.original.shift_x, self.original.shift_y)
    }
}

impl App {
    /// Ouvre l'éditeur pour le FRM affiché (un FRM d'une archive .DAT est lu dans l'archive ; on l'enregistre ailleurs).
    pub fn open_frm_editor(&mut self) {
        let Some(s) = &self.shown else { return };
        if s.loaded.pic.format != Format::Frm {
            return;
        }
        let path = s.loaded.path.clone();
        let loaded = crate::dat::read_any(&path).map_err(|e| e.to_string()).and_then(|data| frm::read_header(&data).map(|h| (data, h)).map_err(|e| e.to_string()));
        match loaded {
            Ok((data, h)) => {
                self.frm_dlg = Some(FrmDlg { path, data, fps: h.fps, action: h.action_frame, shift_x: h.shift_x, shift_y: h.shift_y, original: h, error: None });
            }
            Err(e) => self.toast(i18n_failed(&e)),
        }
    }

    /// Enregistre le FRM modifié sous un nom choisi dans une boîte de dialogue ; renvoie vrai si le fichier a été écrit.
    fn save_frm(&mut self, ctx: &egui::Context, dlg: &mut FrmDlg) -> bool {
        let patched = match frm::patch_header(&dlg.data, dlg.fps, dlg.action, dlg.shift_x, dlg.shift_y) {
            Ok(v) => v,
            Err(e) => {
                dlg.error = Some(e.to_string());
                return false;
            }
        };
        let mut d = rfd::FileDialog::new().add_filter("FRM", &["frm", "FRM"]).set_title(tr("frm.save_as"));
        if let Some(dir) = crate::dat::real_parent(&dlg.path) {
            d = d.set_directory(dir);
        }
        if let Some(name) = dlg.path.file_name() {
            d = d.set_file_name(name.to_string_lossy());
        }
        let Some(target) = d.save_file() else { return false };
        match std::fs::write(&target, patched) {
            Ok(()) => {
                self.toast(trf("frm.saved", &[("file", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]));
                // Le fichier affiché a peut-être été remplacé : on le relit.
                self.cache.clear();
                if self.tree.selected_dir.as_deref() == target.parent() {
                    self.reload_dir(ctx);
                } else if let Some(w) = self.wanted.clone() {
                    self.show_file(ctx, &w);
                }
                true
            }
            Err(e) => {
                dlg.error = Some(e.to_string());
                false
            }
        }
    }

    pub fn frm_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.frm_dlg.take() else { return };
        let (mut save, mut close, mut reset) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("frm_editor")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(460.0);
            crate::dialog::setup(ui);
            close |= crate::dialog::title(ui, &tr("frm.title"));
            ui.label(egui::RichText::new(dlg.path.display().to_string()).weak());
            ui.label(egui::RichText::new(trf("frm.info", &[("dirs", dlg.original.directions.to_string()), ("frames", dlg.original.frames_per_direction.to_string())])).weak());
            ui.add_space(4.0);
            egui::Grid::new("frm_grid").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tr("frm.fps"));
                ui.add(egui::DragValue::new(&mut dlg.fps).range(0..=u16::MAX));
                ui.end_row();
                ui.label(tr("frm.action"));
                let last = dlg.original.frames_per_direction.saturating_sub(1);
                ui.add(egui::DragValue::new(&mut dlg.action).range(0..=last));
                ui.end_row();
            });
            ui.add_space(2.0);
            ui.label(egui::RichText::new(tr("frm.shifts")).strong());
            egui::Grid::new("frm_shifts").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new(tr("frm.shift_x")).weak());
                ui.label(egui::RichText::new(tr("frm.shift_y")).weak());
                ui.end_row();
                for d in 0..dlg.original.directions {
                    ui.label(trf("frm.direction", &[("n", (d + 1).to_string())]));
                    ui.add(egui::DragValue::new(&mut dlg.shift_x[d]));
                    ui.add(egui::DragValue::new(&mut dlg.shift_y[d]));
                    ui.end_row();
                }
            });
            if let Some(e) = &dlg.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                save = crate::dialog::button(ui, tr("frm.save_as")).clicked();
                reset = crate::dialog::button_enabled(ui, dlg.changed(), tr("frm.reset")).clicked();
                close |= crate::dialog::button(ui, tr("c.close")).clicked();
            });
        });
        if reset {
            dlg.fps = dlg.original.fps;
            dlg.action = dlg.original.action_frame;
            dlg.shift_x = dlg.original.shift_x;
            dlg.shift_y = dlg.original.shift_y;
            dlg.error = None;
        }
        if save && self.save_frm(ctx, &mut dlg) {
            close = true;
        }
        if !(close || modal.should_close()) {
            self.frm_dlg = Some(dlg);
        }
    }
}

fn i18n_failed(error: &str) -> String {
    trf("t.op_failed", &[("error", error.to_string())])
}
