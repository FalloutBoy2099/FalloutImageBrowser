//! Fenêtre « Assembler un FRM » : une liste d'images (ordre de lecture) devient un FRM animé de 1 à 6 directions.

use crate::app::App;
use crate::assemble::{self, AssembleOptions};
use crate::i18n::{tr, trf};
use crate::loader::PalPolicy;
use eframe::egui;
use std::path::PathBuf;

pub struct AsmDlg {
    pub files: Vec<PathBuf>,
    pub opt: AssembleOptions,
    /// Utiliser la palette courante (si elle est personnalisée) plutôt que celle de Fallout.
    pub use_current: bool,
    pub error: Option<String>,
}

impl AsmDlg {
    pub fn new() -> AsmDlg {
        AsmDlg { files: Vec::new(), opt: AssembleOptions::default(), use_current: false, error: None }
    }
}

impl Default for AsmDlg {
    fn default() -> Self {
        Self::new()
    }
}

const IMAGE_EXTS: &[&str] = &["png", "gif", "bmp", "tga", "pcx", "jpg", "jpeg", "tif", "tiff", "webp", "qoi", "frm", "fr0", "fr1", "fr2", "fr3", "fr4", "fr5", "art", "rix"];

impl App {
    pub fn open_assemble(&mut self) {
        let mut dlg = AsmDlg::new();
        dlg.use_current = matches!(self.policy, PalPolicy::Custom(..));
        self.asm_dlg = Some(dlg);
    }

    fn save_assembled(&mut self, ctx: &egui::Context, dlg: &mut AsmDlg) -> bool {
        let palette = match (&self.policy, dlg.use_current) {
            (PalPolicy::Custom(p, _), true) => Some(p.clone()),
            _ => None,
        };
        let mut opt = dlg.opt.clone();
        opt.palette = palette;
        let bytes = match assemble::load_frames(&dlg.files).and_then(|frames| assemble::assemble_frm(frames, &opt)) {
            Ok(b) => b,
            Err(e) => {
                dlg.error = Some(e.to_string());
                return false;
            }
        };
        let first = &dlg.files[0];
        let mut d = rfd::FileDialog::new().add_filter("FRM", &["frm", "FRM"]).set_title(tr("asm.save"));
        if let Some(dir) = crate::dat::real_parent(first) {
            d = d.set_directory(dir);
        }
        d = d.set_file_name(format!("{}.frm", crate::batch::stem_of(first)));
        let Some(target) = d.save_file() else { return false };
        match std::fs::write(&target, bytes) {
            Ok(()) => {
                self.toast(trf("asm.saved", &[("file", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]));
                self.cache.clear();
                if self.tree.selected_dir.as_deref() == target.parent() {
                    self.reload_dir(ctx);
                }
                true
            }
            Err(e) => {
                dlg.error = Some(e.to_string());
                false
            }
        }
    }

    pub fn assemble_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.asm_dlg.take() else { return };
        let (mut save, mut close) = (false, false);
        let mut action: Option<(usize, i32)> = None;
        let current_name = match &self.policy {
            PalPolicy::Custom(_, n) => Some(n.clone()),
            _ => None,
        };
        let n = dlg.files.len();
        let divisible = n > 0 && n % dlg.opt.directions == 0;
        let modal = egui::Modal::new(egui::Id::new("assemble_modal")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            crate::dialog::setup(ui);
            close |= crate::dialog::title(ui, &tr("asm.title"));
            if dlg.files.is_empty() {
                ui.label(egui::RichText::new(tr("asm.empty")).weak());
            }
            egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                for (i, f) in dlg.files.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{:>3}", i + 1)).weak().monospace());
                        if ui.small_button("↑").clicked() {
                            action = Some((i, -1));
                        }
                        if ui.small_button("↓").clicked() {
                            action = Some((i, 1));
                        }
                        if ui.small_button("×").clicked() {
                            action = Some((i, 0));
                        }
                        ui.label(f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).on_hover_text(f.display().to_string());
                    });
                }
            });
            ui.horizontal(|ui| {
                if crate::dialog::button(ui, tr("asm.add")).clicked()
                    && let Some(files) = rfd::FileDialog::new().add_filter("images", IMAGE_EXTS).set_title(tr("asm.add")).pick_files()
                {
                    dlg.files.extend(files);
                    dlg.error = None;
                }
                if crate::dialog::button_enabled(ui, !dlg.files.is_empty(), tr("asm.clear")).clicked() {
                    dlg.files.clear();
                }
            });
            ui.separator();
            egui::Grid::new("asm_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tr("asm.directions"));
                ui.add(egui::DragValue::new(&mut dlg.opt.directions).range(1..=6));
                ui.end_row();
                ui.label(tr("asm.fps"));
                ui.add(egui::DragValue::new(&mut dlg.opt.fps).range(1..=255));
                ui.end_row();
                ui.label(tr("asm.action"));
                let last = (n / dlg.opt.directions.max(1)).saturating_sub(1) as u32;
                dlg.opt.action_frame = dlg.opt.action_frame.min(last);
                ui.add(egui::DragValue::new(&mut dlg.opt.action_frame).range(0..=last));
                ui.end_row();
                ui.label(tr("asm.palette"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut dlg.use_current, false, tr("asm.pal_fallout"));
                    if let Some(name) = &current_name {
                        ui.radio_value(&mut dlg.use_current, true, trf("asm.pal_current", &[("name", name.clone())]));
                    }
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut dlg.opt.dither, tr("asm.dither"));
                ui.end_row();
            });
            if n > 0 {
                if divisible {
                    ui.label(
                        egui::RichText::new(trf("asm.summary", &[("n", n.to_string()), ("dirs", dlg.opt.directions.to_string()), ("fpd", (n / dlg.opt.directions).to_string())]))
                            .weak(),
                    );
                } else {
                    ui.colored_label(ui.visuals().error_fg_color, trf("asm.not_divisible", &[("n", n.to_string()), ("dirs", dlg.opt.directions.to_string())]));
                }
            }
            if let Some(e) = &dlg.error {
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                save = crate::dialog::button_enabled(ui, divisible, tr("asm.save")).clicked();
                close |= crate::dialog::button(ui, tr("c.close")).clicked();
            });
        });
        if let Some((i, step)) = action {
            match step {
                0 => {
                    dlg.files.remove(i);
                }
                -1 if i > 0 => dlg.files.swap(i, i - 1),
                1 if i + 1 < dlg.files.len() => dlg.files.swap(i, i + 1),
                _ => {}
            }
        }
        if save && self.save_assembled(ctx, &mut dlg) {
            close = true;
        }
        if !(close || modal.should_close()) {
            self.asm_dlg = Some(dlg);
        }
    }
}
