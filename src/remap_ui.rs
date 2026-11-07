//! Fenêtre « Remapper vers une autre palette » : avant / après côte à côte, puis enregistrement.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::model::*;
use crate::remap;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::sync::Arc;

pub struct RemapDlg {
    source: Arc<crate::loader::Loaded>,
    /// Palette cible : Fallout si `None`.
    target_file: Option<(Arc<Palette>, String)>,
    result: Option<Picture>,
    changed: usize,
    before: Option<TextureHandle>,
    after: Option<TextureHandle>,
    error: Option<String>,
    dirty: bool,
}

fn preview(ctx: &egui::Context, name: &str, pic: &Picture) -> Option<TextureHandle> {
    let (w, h, rgba) = crate::view::render_rgba(pic, 0, 0);
    if w == 0 || h == 0 {
        return None;
    }
    let img = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    Some(ctx.load_texture(name, img, TextureOptions::NEAREST))
}

impl App {
    pub fn open_remap(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.shown else { return };
        let source = s.loaded.clone();
        let mut dlg = RemapDlg { source, target_file: None, result: None, changed: 0, before: None, after: None, error: None, dirty: true };
        if dlg.source.pic.palette.is_none() || dlg.source.pic.frames.iter().any(|f| !matches!(f.pixels, Pixels::Indexed(_))) {
            self.toast(tr("rmp.not_indexed"));
            return;
        }
        dlg.before = preview(ctx, "remap-before", &dlg.source.pic);
        self.remap_dlg = Some(dlg);
    }

    fn save_remapped(&mut self, ctx: &egui::Context, dlg: &mut RemapDlg) -> bool {
        let Some(result) = &dlg.result else { return false };
        let (bytes, ext) = match remap::encode_remapped(result) {
            Ok(x) => x,
            Err(e) => {
                dlg.error = Some(e.to_string());
                return false;
            }
        };
        let mut d = rfd::FileDialog::new().add_filter(ext.to_uppercase(), &[ext]).set_title(tr("rmp.save"));
        if let Some(dir) = crate::dat::real_parent(&dlg.source.path) {
            d = d.set_directory(dir);
        }
        d = d.set_file_name(format!("{}_remap.{ext}", crate::batch::stem_of(&dlg.source.path)));
        let Some(target) = d.save_file() else { return false };
        match std::fs::write(&target, bytes) {
            Ok(()) => {
                self.toast(trf("rmp.saved", &[("file", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]));
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

    pub fn remap_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.remap_dlg.take() else { return };
        if dlg.dirty {
            dlg.dirty = false;
            let target: Arc<Palette> = match &dlg.target_file {
                Some((p, _)) => p.clone(),
                None => crate::palette::fallout(),
            };
            match remap::remap_picture(&dlg.source.pic, &target) {
                Ok(r) => {
                    dlg.changed = remap::changed_pixels(&dlg.source.pic, &r);
                    dlg.after = preview(ctx, "remap-after", &r);
                    dlg.result = Some(r);
                    dlg.error = None;
                }
                Err(e) => {
                    dlg.result = None;
                    dlg.after = None;
                    dlg.error = Some(e.to_string());
                }
            }
        }
        let (mut save, mut close, mut pick) = (false, false, false);
        let mut use_fallout = false;
        let modal = egui::Modal::new(egui::Id::new("remap_modal")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            crate::dialog::setup(ui);
            close |= crate::dialog::title(ui, &tr("rmp.title"));
            ui.label(egui::RichText::new(dlg.source.path.display().to_string()).weak());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tr("rmp.target"));
                if ui.selectable_label(dlg.target_file.is_none(), tr("rmp.fallout")).clicked() {
                    use_fallout = true;
                }
                let name = dlg.target_file.as_ref().map(|(_, n)| n.clone()).unwrap_or_else(|| tr("rmp.file"));
                if ui.selectable_label(dlg.target_file.is_some(), name).clicked() {
                    pick = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                for (label, tex) in [(tr("rmp.before"), &dlg.before), (tr("rmp.after"), &dlg.after)] {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(label).strong());
                        if let Some(t) = tex {
                            let sz = t.size_vec2();
                            let k = (240.0 / sz.x.max(sz.y)).clamp(0.01, 8.0);
                            ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), sz * k)));
                        }
                    });
                }
            });
            if dlg.result.is_some() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(trf("rmp.changed", &[("n", dlg.changed.to_string())])).weak());
            }
            if let Some(e) = &dlg.error {
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                save = crate::dialog::button_enabled(ui, dlg.result.is_some(), tr("rmp.save")).clicked();
                close |= crate::dialog::button(ui, tr("c.close")).clicked();
            });
        });
        if use_fallout && dlg.target_file.is_some() {
            dlg.target_file = None;
            dlg.dirty = true;
        }
        if pick && let Some(path) = rfd::FileDialog::new().add_filter("PAL", &["pal", "PAL"]).set_title(tr("rmp.file")).pick_file() {
            match crate::loader::load_pal_file(&path) {
                Ok(p) => {
                    dlg.target_file = Some((Arc::new(p), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
                    dlg.dirty = true;
                }
                Err(e) => dlg.error = Some(e.to_string()),
            }
        }
        if save {
            self.save_remapped(ctx, &mut dlg);
        }
        if !(close || modal.should_close()) {
            self.remap_dlg = Some(dlg);
        }
    }
}
