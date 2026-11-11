//! Fenêtre « Comparer avec une autre image » : côte à côte ou image des différences.
//! Le chargement de la seconde image et le calcul des différences se font dans un thread.

use crate::app::App;
use crate::compare;
use crate::i18n::{tr, trf};
use crate::loader::{Loaded, PalPolicy, load_file};
use crate::model::*;
use eframe::egui::{self, TextureHandle};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Side,
    Diff,
}

/// Seconde image chargée, ses pixels et les différences avec la première.
struct Other {
    loaded: Loaded,
    size: (usize, usize),
    rgba: Vec<u8>,
    diff: compare::Diff,
}

pub struct CompareDlg {
    left: Arc<Loaded>,
    /// Pixels RVBA de la première image (largeur, hauteur, pixels), partagés avec le thread de calcul.
    left_px: Arc<(usize, usize, Vec<u8>)>,
    right: Option<Arc<Loaded>>,
    mode: Mode,
    tex_left: Option<TextureHandle>,
    tex_right: Option<TextureHandle>,
    tex_diff: Option<TextureHandle>,
    diff: Option<compare::Diff>,
    error: Option<String>,
    job: Option<Receiver<Result<Other, String>>>,
}

/// Pixels RVBA de la première image de la première direction.
fn first_rgba(pic: &Picture) -> (usize, usize, Vec<u8>) {
    crate::view::render_rgba(pic, 0, 0)
}

impl App {
    pub fn open_compare(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.shown else { return };
        let (w, h, rgba) = first_rgba(&s.loaded.pic);
        let tex_left = crate::dialog::preview_texture(ctx, "cmp-left", w, h, rgba.clone());
        self.cmp_dlg = Some(CompareDlg {
            left: s.loaded.clone(),
            left_px: Arc::new((w, h, rgba)),
            right: None,
            mode: Mode::Side,
            tex_left,
            tex_right: None,
            tex_diff: None,
            diff: None,
            error: None,
            job: None,
        });
    }

    /// Choix de la seconde image, puis chargement et comparaison en arrière-plan.
    fn choose_other(&mut self, ctx: &egui::Context, dlg: &mut CompareDlg) {
        let mut d = rfd::FileDialog::new().set_title(tr("cmp.choose"));
        if let Some(dir) = crate::dat::real_parent(&dlg.left.path) {
            d = d.set_directory(dir);
        }
        let Some(path) = d.pick_file() else { return };
        let (policy, left, ctx2): (PalPolicy, _, _) = (self.policy.clone(), dlg.left_px.clone(), ctx.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let run = || -> Result<Other, String> {
                let loaded = load_file(&path, &policy)?;
                let (rw, rh, rgba) = first_rgba(&loaded.pic);
                let diff = compare::diff(&left.2, left.0, left.1, &rgba, rw, rh);
                Ok(Other { loaded, size: (rw, rh), rgba, diff })
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("internal error".into()));
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        dlg.error = None;
        dlg.job = Some(rx);
    }

    pub fn compare_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.cmp_dlg.take() else { return };
        if let Some(rx) = &dlg.job {
            match rx.try_recv() {
                Ok(Ok(o)) => {
                    dlg.job = None;
                    dlg.tex_right = crate::dialog::preview_texture(ctx, "cmp-right", o.size.0, o.size.1, o.rgba);
                    dlg.tex_diff = crate::dialog::preview_texture(ctx, "cmp-diff", o.diff.width, o.diff.height, o.diff.rgba.clone());
                    dlg.diff = Some(o.diff);
                    dlg.right = Some(Arc::new(o.loaded));
                }
                Ok(Err(e)) => {
                    dlg.job = None;
                    dlg.error = Some(e);
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => dlg.job = None,
            }
        }
        let busy = dlg.job.is_some();
        let (mut close, mut pick) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("compare_modal")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(860.0);
            crate::dialog::setup(ui);
            close |= crate::dialog::title(ui, &tr("cmp.title"));
            ui.horizontal(|ui| {
                pick = crate::dialog::button_enabled(ui, !busy, tr("cmp.choose")).clicked();
                ui.add_enabled_ui(dlg.right.is_some(), |ui| {
                    ui.selectable_value(&mut dlg.mode, Mode::Side, tr("cmp.mode_side"));
                    ui.selectable_value(&mut dlg.mode, Mode::Diff, tr("cmp.mode_diff"));
                });
                if busy {
                    ui.spinner();
                }
            });
            ui.add_space(4.0);
            let name = |l: &Loaded| l.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let draw = |ui: &mut egui::Ui, label: String, tex: &Option<TextureHandle>, max: egui::Vec2| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(label).strong());
                    if let Some(t) = tex {
                        let sz = t.size_vec2();
                        let k = (max.x / sz.x).min(max.y / sz.y).clamp(0.01, 8.0);
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), sz * k)));
                    }
                });
            };
            match (&dlg.right, dlg.mode) {
                (None, _) => {
                    ui.horizontal_top(|ui| {
                        draw(ui, name(&dlg.left), &dlg.tex_left, egui::vec2(400.0, 360.0));
                        ui.label(egui::RichText::new(trf("cmp.pick_hint", &[("name", name(&dlg.left))])).weak());
                    });
                }
                (Some(r), Mode::Side) => {
                    ui.horizontal_top(|ui| {
                        draw(ui, name(&dlg.left), &dlg.tex_left, egui::vec2(410.0, 360.0));
                        draw(ui, name(r), &dlg.tex_right, egui::vec2(410.0, 360.0));
                    });
                }
                (Some(_), Mode::Diff) => draw(ui, tr("cmp.mode_diff"), &dlg.tex_diff, egui::vec2(840.0, 360.0)),
            }
            if let Some(d) = &dlg.diff {
                ui.add_space(4.0);
                if !d.same_size {
                    let (a, b) = (dlg.left.pic.frames.first(), dlg.right.as_ref().and_then(|r| r.pic.frames.first()));
                    if let (Some(a), Some(b)) = (a, b) {
                        ui.colored_label(ui.visuals().warn_fg_color, trf("cmp.size_differs", &[("a", format!("{}×{}", a.width, a.height)), ("b", format!("{}×{}", b.width, b.height))]));
                    }
                }
                if d.differing == 0 {
                    ui.label(egui::RichText::new(tr(if d.same_size { "cmp.identical" } else { "cmp.common_identical" })).weak());
                } else {
                    ui.label(egui::RichText::new(trf("cmp.stats", &[("n", d.differing.to_string()), ("total", d.total.to_string()), ("pct", format!("{:.2}", d.percent())), ("max", d.max_delta.to_string())])).weak());
                }
            }
            if let Some(e) = &dlg.error {
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                close |= crate::dialog::button(ui, tr("c.close")).clicked();
            });
        });
        if pick {
            self.choose_other(ctx, &mut dlg);
        }
        if !(close || modal.should_close()) {
            self.cmp_dlg = Some(dlg);
        }
    }
}
