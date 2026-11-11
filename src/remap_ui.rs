//! Fenêtre « Remapper vers une autre palette » : avant / après côte à côte, puis enregistrement.
//! Le calcul du remappage et l'écriture se font dans un thread : la fenêtre reste réactive.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::model::*;
use crate::remap;
use eframe::egui::{self, TextureHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Résultat d'un calcul de remappage : image, nombre de pixels qui changent d'index et aperçu RVBA.
struct Computed {
    result: Picture,
    changed: usize,
    preview: (usize, usize, Vec<u8>),
}

pub struct RemapDlg {
    source: Arc<crate::loader::Loaded>,
    /// Palette cible : Fallout si `None`.
    target_file: Option<(Arc<Palette>, String)>,
    result: Option<Picture>,
    changed: usize,
    before: Option<TextureHandle>,
    after: Option<TextureHandle>,
    error: Option<String>,
    /// La palette cible a changé : le remappage est à refaire.
    dirty: bool,
    compute: Option<Receiver<Result<Computed, String>>>,
    save_job: Option<Receiver<Result<PathBuf, String>>>,
}

impl App {
    pub fn open_remap(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.shown else { return };
        let source = s.loaded.clone();
        if source.pic.palette.is_none() || source.pic.frames.iter().any(|f| !matches!(f.pixels, Pixels::Indexed(_))) {
            self.toast(tr("rmp.not_indexed"));
            return;
        }
        let (w, h, rgba) = crate::view::render_rgba(&source.pic, 0, 0);
        let before = crate::dialog::preview_texture(ctx, "remap-before", w, h, rgba);
        self.remap_dlg = Some(RemapDlg { source, target_file: None, result: None, changed: 0, before, after: None, error: None, dirty: true, compute: None, save_job: None });
    }

    /// Lance le calcul du remappage vers la palette cible choisie.
    fn start_remap(ctx: &egui::Context, dlg: &mut RemapDlg) {
        let target: Arc<Palette> = match &dlg.target_file {
            Some((p, _)) => p.clone(),
            None => crate::palette::fallout(),
        };
        let (source, ctx2) = (dlg.source.clone(), ctx.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let run = || -> Result<Computed, String> {
                let result = remap::remap_picture(&source.pic, &target).map_err(|e| e.to_string())?;
                let changed = remap::changed_pixels(&source.pic, &result);
                let preview = crate::view::render_rgba(&result, 0, 0);
                Ok(Computed { result, changed, preview })
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("internal error".into()));
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        dlg.dirty = false;
        dlg.compute = Some(rx);
    }

    /// Demande où écrire, puis encode et écrit en arrière-plan (avec le `.PAL` cible à côté d'un FRM).
    fn start_save(ctx: &egui::Context, dlg: &mut RemapDlg) {
        let Some(result) = dlg.result.clone() else { return };
        let ext = if result.format == Format::Frm { "frm" } else { "png" };
        let mut d = rfd::FileDialog::new().add_filter(ext.to_uppercase(), &[ext]).set_title(tr("rmp.save"));
        if let Some(dir) = crate::dat::real_parent(&dlg.source.path) {
            d = d.set_directory(dir);
        }
        d = d.set_file_name(format!("{}_remap.{ext}", crate::batch::stem_of(&dlg.source.path)));
        let Some(target) = d.save_file() else { return };
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let run = || -> Result<PathBuf, String> {
                let out = remap::encode_remapped(&result).map_err(|e| e.to_string())?;
                std::fs::write(&target, &out.bytes).map_err(|e| e.to_string())?;
                // Un FRM n'embarque pas sa palette : le .PAL cible est écrit à côté, sans remplacer un fichier existant.
                if let Some(pal) = &out.pal {
                    let p = target.with_extension("pal");
                    if !p.exists() {
                        std::fs::write(&p, pal).map_err(|e| e.to_string())?;
                    }
                }
                Ok(target.clone())
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("internal error".into()));
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        dlg.error = None;
        dlg.save_job = Some(rx);
    }

    pub fn remap_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.remap_dlg.take() else { return };
        // Résultats des travaux en arrière-plan.
        if let Some(rx) = &dlg.compute {
            match rx.try_recv() {
                Ok(Ok(c)) => {
                    dlg.compute = None;
                    dlg.changed = c.changed;
                    let (w, h, rgba) = c.preview;
                    dlg.after = crate::dialog::preview_texture(ctx, "remap-after", w, h, rgba);
                    dlg.result = Some(c.result);
                    dlg.error = None;
                }
                Ok(Err(e)) => {
                    dlg.compute = None;
                    dlg.result = None;
                    dlg.after = None;
                    dlg.error = Some(e);
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => dlg.compute = None,
            }
        }
        if let Some(rx) = &dlg.save_job {
            match rx.try_recv() {
                Ok(Ok(target)) => {
                    dlg.save_job = None;
                    self.toast(trf("rmp.saved", &[("file", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]));
                    if self.tree.selected_dir.as_deref() == target.parent() {
                        self.reload_dir(ctx);
                    }
                }
                Ok(Err(e)) => {
                    dlg.save_job = None;
                    dlg.error = Some(e);
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => dlg.save_job = None,
            }
        }
        if dlg.dirty && dlg.compute.is_none() {
            Self::start_remap(ctx, &mut dlg);
        }
        let busy = dlg.compute.is_some() || dlg.save_job.is_some();
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
                save = crate::dialog::button_enabled(ui, dlg.result.is_some() && !busy, tr("rmp.save")).clicked();
                close |= crate::dialog::button(ui, tr("c.close")).clicked();
                if busy {
                    ui.spinner();
                }
            });
        });
        if use_fallout && dlg.target_file.is_some() {
            dlg.target_file = None;
            dlg.dirty = true;
        }
        if pick
            && let Some(path) = rfd::FileDialog::new().add_filter("PAL", &["pal", "PAL"]).set_title(tr("rmp.file")).pick_file()
        {
            match crate::loader::load_pal_file(&path) {
                Ok(p) => {
                    dlg.target_file = Some((Arc::new(p), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
                    dlg.dirty = true;
                }
                Err(e) => dlg.error = Some(e),
            }
        }
        if save {
            Self::start_save(ctx, &mut dlg);
        }
        if !(close || modal.should_close()) {
            self.remap_dlg = Some(dlg);
        }
    }
}
