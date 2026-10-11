//! Fenêtre modale « Extraire la palette » : palette de l'image courante vers un fichier de palette.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::palette_export::{self, PalFormat};
use eframe::egui;

pub struct PalDlg {
    pub open: bool,
    pub fmt: PalFormat,
    pub error: Option<String>,
}

impl PalDlg {
    pub fn new(id: &str) -> PalDlg {
        PalDlg { open: false, fmt: PalFormat::from_id(id).unwrap_or(PalFormat::FalloutPal), error: None }
    }
}

impl App {
    pub fn open_palette_dialog(&mut self) {
        if self.shown.is_some() {
            self.pal_dlg.open = true;
            self.pal_dlg.error = None;
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
        let ncolors = palette_export::encode(self.pal_dlg.fmt, &loaded.pic, &stem).ok().map(|_| {
            let ex = palette_export::extract(&loaded.pic, self.pal_dlg.fmt == PalFormat::FalloutPal);
            (0..256).filter(|&i| ex.palette.used[i] && ex.palette.colors[i][3] > 0).count()
        });
        let modal = egui::Modal::new(egui::Id::new("palette_modal")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(tr("x.title"));
            ui.label(egui::RichText::new(loaded.path.display().to_string()).weak().small());
            ui.add_space(6.0);
            egui::Grid::new("pal_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(tr("x.format"));
                egui::ComboBox::from_id_salt("pal_fmt").selected_text(self.pal_dlg.fmt.label()).width(260.0).show_ui(ui, |ui| {
                    for f in PalFormat::ALL {
                        ui.selectable_value(&mut self.pal_dlg.fmt, f, f.label());
                    }
                });
                ui.end_row();
                if let Some(n) = ncolors {
                    ui.label("");
                    ui.label(egui::RichText::new(trf("x.count", &[("n", n.to_string())])).weak());
                    ui.end_row();
                }
            });
            if let Some(e) = &self.pal_dlg.error {
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(tr("x.save")).clicked() {
                    save = true;
                }
                if ui.button(tr("c.cancel")).clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if save {
            let fmt = self.pal_dlg.fmt;
            let mut d = rfd::FileDialog::new().set_title(tr("x.title")).add_filter(fmt.label(), &[fmt.ext()]).set_file_name(format!("{stem}.{}", fmt.ext()));
            if let Some(dir) = self.settings.pal_dir.clone().or_else(|| crate::dat::real_parent(&loaded.path)) {
                d = d.set_directory(dir);
            }
            if let Some(mut path) = d.save_file() {
                if path.extension().is_none() {
                    path.set_extension(fmt.ext());
                }
                let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(stem.clone());
                match palette_export::encode(fmt, &loaded.pic, &name).and_then(|b| std::fs::write(&path, b).map_err(Into::into)) {
                    Ok(()) => {
                        self.settings.pal_dir = path.parent().map(|p| p.to_path_buf());
                        self.settings.pal_fmt = fmt.id().to_string();
                        let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        self.pal_dlg.open = false;
                        self.toast(trf("x.done", &[("file", file)]));
                        // Rafraîchit la liste si le fichier est écrit dans le dossier courant.
                        if let (Some(dir), Some(sel)) = (path.parent(), self.tree.selected_dir.clone()) {
                            if dir == sel {
                                self.reload_dir_keep_view();
                            }
                        }
                    }
                    Err(e) => self.pal_dlg.error = Some(e.to_string()),
                }
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
