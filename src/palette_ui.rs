//! Fenêtre modale « Extraire la palette » : palette de l'image courante vers un fichier de palette.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::palette_export::{self, PalFormat};
use eframe::egui;

pub struct PalDlg {
    pub open: bool,
    pub fmt: PalFormat,
    pub error: Option<String>,
    /// Nom du fichier (sans extension) et dossier de sortie en cours de saisie.
    pub name: String,
    pub out_dir: String,
}

impl PalDlg {
    pub fn new(id: &str) -> PalDlg {
        PalDlg { open: false, fmt: PalFormat::from_id(id).unwrap_or(PalFormat::FalloutPal), error: None, name: String::new(), out_dir: String::new() }
    }
}

impl App {
    pub fn open_palette_dialog(&mut self) {
        if self.shown.is_some() {
            self.pal_dlg.open = true;
            self.pal_dlg.error = None;
            if let Some(s) = &self.shown {
                self.pal_dlg.name = crate::batch::stem_of(&s.loaded.path);
                let dir = self.settings.pal_dir.clone().or_else(|| crate::dat::real_parent(&s.loaded.path));
                self.pal_dlg.out_dir = dir.map(|d| d.display().to_string()).unwrap_or_default();
            }
        }
    }

    pub fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.pal_dlg.open {
            return;
        }
        let Some(loaded) = self.shown.as_ref().map(|s| s.loaded.clone()) else {
            self.pal_dlg.open = false;
            return;
        };
        let stem = crate::batch::stem_of(&loaded.path);
        let (mut save, mut close) = (false, false);
        let name = self.pal_dlg.name.trim().to_string();
        let out_dir = std::path::PathBuf::from(self.pal_dlg.out_dir.trim());
        let invalid = crate::fileops::validate_name(&name).is_err();
        let target = out_dir.join(format!("{name}.{}", self.pal_dlg.fmt.ext()));
        let exists = !invalid && target.exists();
        let can_save = !invalid && !exists && !self.pal_dlg.out_dir.trim().is_empty();
        let ncolors = palette_export::encode(self.pal_dlg.fmt, &loaded.pic, &stem).ok().map(|_| {
            let ex = palette_export::extract(&loaded.pic, self.pal_dlg.fmt == PalFormat::FalloutPal);
            (0..256).filter(|&i| ex.palette.used[i] && ex.palette.colors[i][3] > 0).count()
        });
        let modal = egui::Modal::new(egui::Id::new("palette_modal")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(480.0);
            crate::dialog::setup(ui);
            if crate::dialog::title(ui, &tr("x.title")) {
                close = true;
            }
            ui.label(egui::RichText::new(loaded.path.display().to_string()).weak());
            ui.add_space(6.0);
            egui::Grid::new("pal_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tr("x.format"));
                egui::ComboBox::from_id_salt("pal_fmt").selected_text(self.pal_dlg.fmt.label()).width(260.0).show_ui(ui, |ui| {
                    for f in PalFormat::ALL {
                        ui.selectable_value(&mut self.pal_dlg.fmt, f, f.label());
                    }
                });
                ui.end_row();
                ui.label(tr("c.name"));
                ui.add(egui::TextEdit::singleline(&mut self.pal_dlg.name).desired_width(f32::INFINITY));
                ui.end_row();
                ui.label(tr("c.out_dir"));
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 36.0;
                    ui.add(egui::TextEdit::singleline(&mut self.pal_dlg.out_dir).desired_width(w));
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new().set_title(tr("c.out_dir")).set_directory(&out_dir).pick_folder()
                    {
                        self.pal_dlg.out_dir = p.display().to_string();
                    }
                });
                ui.end_row();
                if let Some(n) = ncolors {
                    ui.label("");
                    ui.label(egui::RichText::new(trf("x.count", &[("n", n.to_string())])).weak());
                    ui.end_row();
                }
            });
            if exists {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_exists"));
            } else if invalid && !self.pal_dlg.name.trim().is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_invalid"));
            }
            if let Some(e) = &self.pal_dlg.error {
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if crate::dialog::button_enabled(ui, can_save, tr("x.save")).clicked() {
                    save = true;
                }
                if crate::dialog::button(ui, tr("c.cancel")).clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if save {
            let fmt = self.pal_dlg.fmt;
            let written = std::fs::create_dir_all(&out_dir).map_err(anyhow::Error::from).and_then(|()| {
                let bytes = palette_export::encode(fmt, &loaded.pic, &name)?;
                std::fs::write(&target, bytes)?;
                Ok(())
            });
            match written {
                Ok(()) => {
                    self.settings.pal_dir = Some(out_dir.clone());
                    self.settings.pal_fmt = fmt.id().to_string();
                    let file = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    self.pal_dlg.open = false;
                    self.toast(trf("x.done", &[("file", file)]));
                    // Rafraîchit la liste si le fichier est écrit dans le dossier courant.
                    if self.tree.selected_dir.as_deref() == Some(out_dir.as_path()) {
                        self.reload_dir_keep_view();
                    }
                }
                Err(e) => self.pal_dlg.error = Some(e.to_string()),
            }
        }
        if close {
            self.pal_dlg.open = false;
        }
    }

    /// Recharge la liste des fichiers du dossier courant sans toucher à l'image affichée.
    pub fn reload_dir_keep_view(&mut self) {
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.tree.reload_dir(&dir);
            self.files = self.tree.files_of(&dir);
            if let Some(w) = &self.wanted {
                self.index = self.files.iter().position(|f| f == w);
            }
        }
    }
}
