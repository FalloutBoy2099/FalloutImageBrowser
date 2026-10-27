//! Panneau de l'arborescence : favoris, filtre et liste des dossiers / fichiers.

use crate::app::{App, MAX_FAVORITES};
use crate::i18n::tr;
use crate::icons::{Icon, icon_button};
use crate::tree::RowKind;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::widgets::{letter_button, letter_of};

impl App {
    /// Boutons d'accès rapide (8 au plus) : Bureau, Documents, Téléchargements (réservés) puis les favoris ;
    /// clic droit sur un favori : le retirer.
    fn favorites_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut open: Option<std::path::PathBuf> = None;
        let mut remove: Option<usize> = None;
        let mut toggle = false;
        let folder_view = self.settings.folder_view;
        let cur = self.tree.selected_dir.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            // Bascule arborescence / mode dossier (miniatures), puis les accès rapides.
            let tip = tr(if folder_view { "t.tree_view" } else { "t.folder_view" });
            if icon_button(ui, Icon::Thumbs, &tip, folder_view, true).clicked() {
                toggle = true;
            }
            ui.separator();
            for (key, path) in &self.special {
                let tip = format!("{}\n{}", tr(key), path.display());
                let icon = match *key {
                    "t.desktop" => Icon::Desktop,
                    "t.documents" => Icon::Documents,
                    _ => Icon::Downloads,
                };
                if icon_button(ui, icon, &tip, cur.as_ref() == Some(path), true).clicked() {
                    open = Some(path.clone());
                }
            }
            for (i, path) in self.settings.favorites.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
                let tip = format!("{name}\n{}", path.display());
                let r = letter_button(ui, &letter_of(&name), &tip, cur.as_ref() == Some(path));
                if r.clicked() {
                    open = Some(path.clone());
                }
                r.context_menu(|ui| {
                    if ui.button(tr("t.fav_remove")).clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
        });
        if toggle {
            self.settings.folder_view = !self.settings.folder_view;
            if self.settings.folder_view {
                self.folder_follow = true;
            } else {
                // Retour à l'arborescence : elle se place sur le dossier courant.
                self.tree_scroll_to = self.tree.selected_dir.as_ref().and_then(|d| self.tree.rows.iter().position(|r| &r.path == d));
            }
        }
        if let Some(i) = remove {
            self.settings.favorites.remove(i);
        }
        if let Some(p) = open {
            if crate::dat::stat(&p).is_some() || p.is_dir() {
                self.open_dir(ctx, &p, false);
                // Faire défiler jusqu'au dossier choisi (et non jusqu'au fichier affiché, qui peut être ailleurs).
                self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == p);
            } else {
                self.toast(tr("t.folder_missing"));
            }
        }
    }

    pub fn tree_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.tree_rect = Some(ui.max_rect());
        ui.add_space(5.0);
        self.favorites_bar(ui, &ctx);
        // Filtre sur les noms de fichiers du dossier courant.
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            let clear = !self.file_filter.is_empty();
            let w = ui.available_width() - if clear { 30.0 } else { 0.0 };
            let edit = ui.add(egui::TextEdit::singleline(&mut self.file_filter).hint_text(tr("t.filter")).desired_width(w).margin(vec2(8.0, 7.0)));
            let cleared = clear && ui.button("×").clicked();
            if cleared {
                self.file_filter.clear();
            }
            if edit.changed() || cleared {
                self.apply_view_options();
            }
        });
        ui.add_space(5.0);
        if self.settings.folder_view {
            self.folder_panel(ui);
            return;
        }
        // Largeur du contenu : la ligne la plus longue (indentation, icône et nom) ; recalculée quand les lignes changent.
        if self.tree_w.0 != self.tree.rev {
            let font = FontId::proportional(13.5);
            // Mettre en page des dizaines de milliers de noms est lent : seuls les candidats les plus larges (indentation + nombre de
            // caractères, les caractères non ASCII comptant double) sont mesurés.
            let est = |r: &crate::tree::Row| r.depth as f32 * 14.0 + r.name.chars().map(|c| if c.is_ascii() { 1.0 } else { 2.0 }).sum::<f32>() * 7.0;
            let mut cand: Vec<&crate::tree::Row> = self.tree.rows.iter().collect();
            if cand.len() > 64 {
                cand.select_nth_unstable_by(63, |a, b| est(b).total_cmp(&est(a)));
                cand.truncate(64);
            }
            let widest = cand
                .iter()
                .map(|r| 4.0 + r.depth as f32 * 14.0 + 33.0 + ui.painter().layout_no_wrap(r.name.clone(), font.clone(), Color32::WHITE).size().x + 10.0)
                .fold(0.0, f32::max);
            self.tree_w = (self.tree.rev, widest);
        }
        let content_w = self.tree_w.1;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let total = self.tree.rows.len();
        let cur = self.wanted.clone();
        let sel_dir = self.tree.selected_dir.clone();
        let mut action: Option<(usize, bool)> = None; // (ligne, clic sur la flèche)
        let mut ctx_act: Option<(std::path::PathBuf, u8)> = None;
        let favs = self.settings.favorites.clone();
        let fav_free = favs.len() < MAX_FAVORITES;
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(r) = self.tree_scroll_to.take() {
            let target = (r as f32 * row_h - ui.available_height() * 0.5).max(0.0);
            area = area.vertical_scroll_offset(target);
        }
        area.show_rows(ui, row_h, total, |ui, range| {
            // Un contenu plus large que le panneau fait apparaître la barre de défilement horizontale.
            ui.set_min_width(content_w);
            let w = ui.available_width();
            for i in range {
                let row = &self.tree.rows[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click());
                let selected = matches!(row.kind, RowKind::File) && cur.as_deref() == Some(row.path.as_path());
                let is_sel_dir = matches!(row.kind, RowKind::Dir { .. }) && sel_dir.as_deref() == Some(row.path.as_path());
                let vis = ui.visuals();
                if selected {
                    ui.painter().rect_filled(rect, 3.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, vis.widgets.hovered.bg_fill.gamma_multiply(0.6));
                }
                let x0 = rect.left() + 4.0 + row.depth as f32 * 14.0;
                let cy = rect.center().y;
                let text_col = if selected { vis.selection.stroke.color } else { vis.text_color() };
                match row.kind {
                    RowKind::Dir { expanded } => {
                        let c = pos2(x0 + 6.0, cy);
                        let pts = if expanded {
                            vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 3.0)]
                        } else {
                            vec![c + vec2(-2.5, -4.0), c + vec2(-2.5, 4.0), c + vec2(3.0, 0.0)]
                        };
                        ui.painter().add(egui::Shape::convex_polygon(pts, vis.weak_text_color(), Stroke::NONE));
                        // dossier
                        let fx = x0 + 15.0;
                        let fcol = if row.archive {
                            if is_sel_dir { Color32::from_rgb(120, 170, 235) } else { Color32::from_rgb(90, 140, 205) }
                        } else if is_sel_dir {
                            Color32::from_rgb(240, 190, 70)
                        } else {
                            Color32::from_rgb(200, 160, 60)
                        };
                        ui.painter().rect_filled(Rect::from_min_size(pos2(fx, cy - 4.0), vec2(13.0, 9.0)), 1.5, fcol);
                        ui.painter().rect_filled(Rect::from_min_size(pos2(fx, cy - 6.0), vec2(6.0, 3.0)), 1.0, fcol);
                    }
                    RowKind::File => {
                        let fx = x0 + 15.0;
                        let col = if selected { text_col } else { vis.weak_text_color() };
                        ui.painter().rect_stroke(Rect::from_min_size(pos2(fx + 1.0, cy - 6.0), vec2(10.0, 12.0)), 1.5, Stroke::new(1.2, col), egui::StrokeKind::Inside);
                        ui.painter().line_segment([pos2(fx + 3.0, cy + 2.0), pos2(fx + 6.0, cy - 1.0)], Stroke::new(1.0, col));
                        ui.painter().line_segment([pos2(fx + 6.0, cy - 1.0), pos2(fx + 9.0, cy + 2.0)], Stroke::new(1.0, col));
                    }
                }
                let text_x = x0 + 15.0 + 18.0;
                let clip = ui.painter().with_clip_rect(rect);
                let font = FontId::proportional(13.5);
                clip.text(pos2(text_x, cy), Align2::LEFT_CENTER, &row.name, font, if is_sel_dir && !selected { vis.strong_text_color() } else { text_col });
                if resp.clicked() {
                    let on_arrow = matches!(row.kind, RowKind::Dir { .. }) && resp.interact_pointer_pos().is_some_and(|p| p.x < x0 + 14.0);
                    action = Some((i, on_arrow));
                }
                let is_dir = matches!(row.kind, RowKind::Dir { .. });
                resp.context_menu(|ui| {
                    if ui.add_enabled(is_dir, egui::Button::new(tr("t.expand_all"))).clicked() {
                        ctx_act = Some((row.path.clone(), 0));
                        ui.close();
                    }
                    if ui.add_enabled(is_dir, egui::Button::new(tr("t.collapse_children"))).clicked() {
                        ctx_act = Some((row.path.clone(), 7));
                        ui.close();
                    }
                    if ui.button(tr("t.collapse_others")).clicked() {
                        ctx_act = Some((row.path.clone(), 1));
                        ui.close();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new(tr("t.refresh")).shortcut_text("F5")).clicked() {
                        let target = if is_dir { row.path.clone() } else { sel_dir.clone().unwrap_or_else(|| row.path.clone()) };
                        ctx_act = Some((target, 4));
                        ui.close();
                    }
                    // Favoris : grisé pour un fichier, un dossier déjà favori, ou quand les 5 places sont prises.
                    let can_fav = is_dir && fav_free && !favs.contains(&row.path);
                    if ui.add_enabled(can_fav, egui::Button::new(tr("t.fav_add"))).clicked() {
                        ctx_act = Some((row.path.clone(), 5));
                        ui.close();
                    }
                    if ui.button(tr("t.open_explorer")).clicked() {
                        ctx_act = Some((row.path.clone(), 2));
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(crate::dat::is_virtual(&row.path), egui::Button::new(tr("t.extract"))).clicked() {
                        ctx_act = Some((row.path.clone(), 3));
                        ui.close();
                    }
                    // Animation à partir des images du dossier (impossible dans une archive).
                    if ui.add_enabled(is_dir && !crate::dat::is_virtual(&row.path), egui::Button::new(tr("t.create_anim"))).clicked() {
                        ctx_act = Some((row.path.clone(), 6));
                        ui.close();
                    }
                });
            }
        });
        if let Some((path, what)) = ctx_act {
            if what == 0 {
                // Plus de 100 dossiers : confirmation avant de tout déplier.
                if self.tree.count_dirs(&path, 100) > 100 {
                    self.expand_confirm = Some(path);
                } else {
                    self.tree.expand_all(&path);
                }
            } else if what == 2 {
                if let Err(e) = crate::opener::reveal(&path) {
                    self.toast(e);
                }
            } else if what == 3 {
                self.extract_from_archive(&path);
            } else if what == 4 {
                self.refresh_dir(&ctx, &path);
            } else if what == 5 {
                if self.settings.favorites.len() < MAX_FAVORITES && !self.settings.favorites.contains(&path) {
                    self.settings.favorites.push(path);
                }
            } else if what == 6 {
                self.open_anim_dialog(&path);
            } else if what == 7 {
                self.tree.collapse_children(&path);
            } else {
                self.tree.collapse_others(&path);
            }
        }
        if let Some((i, on_arrow)) = action {
            let row = self.tree.rows[i].clone();
            match row.kind {
                RowKind::Dir { .. } => {
                    if on_arrow {
                        self.tree.toggle(&row.path);
                    } else {
                        self.open_dir(&ctx, &row.path, true);
                    }
                }
                RowKind::File => self.show_file(&ctx, &row.path),
            }
        }
    }
}
