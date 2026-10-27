//! Mode dossier du panneau de gauche : dossier parent, sous-dossiers et fichiers du dossier courant en miniatures.
//! Il partage le dossier courant de l'arborescence : changer de dossier dans l'un change dans l'autre.

use crate::app::App;
use crate::i18n::tr;
use crate::thumbs::Thumb;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
use std::path::PathBuf;

/// Marge intérieure d'une cellule de miniature, en points.
const PAD: f32 = 5.0;
/// Hauteur réservée au nom sous la miniature.
const NAME_H: f32 = 18.0;

/// Dessine un dossier (même pictogramme que l'arborescence).
fn folder_icon(p: &egui::Painter, at: egui::Pos2, archive: bool) {
    let col = if archive { Color32::from_rgb(90, 140, 205) } else { Color32::from_rgb(200, 160, 60) };
    p.rect_filled(Rect::from_min_size(pos2(at.x, at.y - 4.0), vec2(13.0, 9.0)), 1.5, col);
    p.rect_filled(Rect::from_min_size(pos2(at.x, at.y - 6.0), vec2(6.0, 3.0)), 1.0, col);
}

enum Nav {
    Dir(PathBuf),
    File(PathBuf),
}

impl App {
    pub fn folder_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        self.thumbs.poll(&ctx);
        self.thumbs.begin_frame();
        let cur = self.tree.selected_dir.clone();
        let parent = cur.as_ref().and_then(|d| d.parent()).filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
        let dirs = self.tree.subdirs_of(cur.as_deref());
        let files = cur.as_ref().map(|d| self.tree.files_of(d)).unwrap_or_default();
        // Dossier affiché : le nom du dossier courant, avec son chemin en infobulle.
        if let Some(d) = &cur {
            let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| d.display().to_string());
            ui.label(egui::RichText::new(name).strong()).on_hover_text(d.display().to_string());
            ui.add_space(2.0);
        }
        // Un changement de fichier affiché (clavier, molette...) fait défiler jusqu'à lui.
        if self.wanted != self.folder_seen {
            self.folder_seen = self.wanted.clone();
            self.folder_follow = true;
        }
        let px = self.settings.thumb_size.px();
        let phys = (px as f32 * ppp).round() as u32;
        let cell_w = px as f32 + 2.0 * PAD;
        let cell_h = px as f32 + 2.0 * PAD + NAME_H;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let selected = self.wanted.clone();
        let mut nav: Option<Nav> = None;
        let mut visible_keys: Vec<crate::thumbs::Key> = Vec::new();
        egui::ScrollArea::vertical().id_salt(("folder_scroll", &cur)).auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            let w = ui.available_width();
            let cols = ((w / cell_w).floor() as usize).max(1);
            let n_dirs = dirs.len() + usize::from(parent.is_some());
            let grid_rows = files.len().div_ceil(cols);
            let gap = if n_dirs > 0 && !files.is_empty() { 6.0 } else { 0.0 };
            let total = n_dirs as f32 * row_h + gap + grid_rows as f32 * cell_h;
            let (area, _) = ui.allocate_exact_size(vec2(w, total.max(1.0)), Sense::hover());
            let (top, bottom) = (viewport.min.y, viewport.max.y);
            // Dossiers (parent d'abord).
            let first = ((top / row_h).floor().max(0.0) as usize).min(n_dirs);
            let last = (((bottom / row_h).ceil().max(0.0)) as usize).min(n_dirs);
            for i in first..last {
                let r = Rect::from_min_size(area.min + vec2(0.0, i as f32 * row_h), vec2(w, row_h));
                let resp = ui.interact(r, ui.id().with(("fdir", i)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 3.0, ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.6));
                }
                let (name, path, archive) = match (&parent, i) {
                    (Some(p), 0) => ("..".to_string(), p.clone(), false),
                    (Some(_), k) => {
                        let d = &dirs[k - 1];
                        (d.1.clone(), d.0.clone(), d.2)
                    }
                    (None, k) => {
                        let d = &dirs[k];
                        (d.1.clone(), d.0.clone(), d.2)
                    }
                };
                folder_icon(ui.painter(), pos2(r.left() + 6.0, r.center().y), archive);
                ui.painter().with_clip_rect(r).text(pos2(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, &name, FontId::proportional(13.5), ui.visuals().text_color());
                if i == 0 && parent.is_some() {
                    resp.clone().on_hover_text(tr("t.parent_dir"));
                }
                if resp.clicked() {
                    nav = Some(Nav::Dir(path));
                }
            }
            // Miniatures.
            let grid_top = n_dirs as f32 * row_h + gap;
            let r0 = (((top - grid_top) / cell_h).floor().max(0.0) as usize).min(grid_rows);
            let r1 = (((bottom - grid_top) / cell_h).ceil().max(0.0) as usize).min(grid_rows);
            // Le fichier affiché doit rester visible : défilement minimal, une seule fois par changement.
            if self.folder_follow
                && let Some(i) = selected.as_ref().and_then(|s| files.iter().position(|f| f == s))
            {
                let r = Rect::from_min_size(area.min + vec2((i % cols) as f32 * cell_w, grid_top + (i / cols) as f32 * cell_h), vec2(cell_w, cell_h));
                ui.scroll_to_rect(r, None);
                self.folder_follow = false;
            }
            for row in r0..r1 {
                for c in 0..cols {
                    let i = row * cols + c;
                    let Some(path) = files.get(i) else { break };
                    let cell = Rect::from_min_size(area.min + vec2(c as f32 * cell_w, grid_top + row as f32 * cell_h), vec2(cell_w, cell_h));
                    let resp = ui.interact(cell, ui.id().with(("fthumb", i)), Sense::click());
                    let is_sel = selected.as_ref() == Some(path);
                    if is_sel {
                        ui.painter().rect_filled(cell.shrink(1.0), 4.0, ui.visuals().selection.bg_fill);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(cell.shrink(1.0), 4.0, ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.6));
                    }
                    let boxr = Rect::from_min_size(cell.min + vec2(PAD, PAD), vec2(px as f32, px as f32));
                    ui.painter().rect_filled(boxr, 3.0, ui.visuals().extreme_bg_color);
                    let key = (path.clone(), phys);
                    visible_keys.push(key.clone());
                    match self.thumbs.get(&key) {
                        Some(Thumb::Ready(tex, [tw, th])) => {
                            // Une image plus petite que la boîte est agrandie d'un facteur entier (pixels nets).
                            let big = (*tw).max(*th) as f32;
                            let k = if big < phys as f32 { (phys as f32 / big).floor().max(1.0) } else { 1.0 };
                            let size = vec2(*tw as f32, *th as f32) * k / ppp;
                            let ir = Rect::from_center_size(boxr.center(), size);
                            ui.painter().image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        Some(Thumb::Failed) => {
                            let ext = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                            ui.painter().text(boxr.center(), Align2::CENTER_CENTER, ext, FontId::proportional(13.0), ui.visuals().weak_text_color());
                        }
                        None => {
                            ui.painter().text(boxr.center(), Align2::CENTER_CENTER, "…", FontId::proportional(16.0), ui.visuals().weak_text_color());
                        }
                    }
                    // Nom : centré s'il tient, sinon aligné à gauche et coupé.
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let col = if is_sel { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() };
                    let galley = ui.painter().layout_no_wrap(name.clone(), FontId::proportional(12.0), col);
                    let avail = cell_w - 2.0 * PAD;
                    let y = boxr.bottom() + 2.0;
                    let clip = ui.painter().with_clip_rect(Rect::from_min_size(pos2(cell.left() + PAD, y), vec2(avail, NAME_H)));
                    let x = if galley.size().x <= avail { cell.center().x - galley.size().x / 2.0 } else { cell.left() + PAD };
                    clip.galley(pos2(x, y), galley, col);
                    let resp = resp.on_hover_text(name);
                    if resp.clicked() {
                        nav = Some(Nav::File(path.clone()));
                    }
                }
            }
        });
        let policy = self.policy.clone();
        self.thumbs.want(visible_keys, &policy);
        match nav {
            Some(Nav::Dir(p)) => self.open_dir(&ctx, &p, true),
            Some(Nav::File(p)) => self.show_file(&ctx, &p),
            None => {}
        }
    }
}
