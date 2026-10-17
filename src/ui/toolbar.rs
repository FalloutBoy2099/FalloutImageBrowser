//! Barre d'outils du bas.

use crate::app::App;
use crate::i18n::tr;
use crate::icons::{Icon, icon_button};
use eframe::egui::{self, Color32, FontId, Ui, Vec2, pos2};

impl App {
    pub fn toolbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(3.0);
        let mut group_right = 0.0f32;
        let bar = ui
            .horizontal(|ui| {
                // Groupe de boutons centré (largeur mesurée à l'image précédente).
                let pad = ((ui.available_width() - self.toolbar_w) / 2.0).max(0.0);
                ui.add_space(pad);
                let group = ui.horizontal(|ui| {
                    // Hauteur de ligne fixée dès le départ : sinon le premier élément (case à cocher) reste en haut.
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
                    ui.separator();
                    if icon_button(ui, Icon::Folder, &format!("{}  (Ctrl+L)", tr("t.folder")), false, true).clicked() {
                        self.open_folder_dialog();
                    }
                    ui.separator();
                    let has_files = self.files.len() > 1;
                    if icon_button(ui, Icon::Prev, &tr("t.prev"), false, has_files).clicked() {
                        self.step(&ctx, -1);
                    }
                    if icon_button(ui, Icon::Next, &tr("t.next"), false, has_files).clicked() {
                        self.step(&ctx, 1);
                    }
                    ui.separator();
                    let has_img = self.shown.is_some();
                    // Ajustement automatique plafonné à 100 % (petite image) : c'est « 100 % » qui est actif.
                    let at_actual = (self.zoom - 1.0).abs() < 1e-3;
                    if icon_button(ui, Icon::ZoomIn, &tr("t.zoom_in"), false, has_img).clicked() {
                        self.zoom_by(1.25);
                    }
                    if icon_button(ui, Icon::ZoomOut, &tr("t.zoom_out"), false, has_img).clicked() {
                        self.zoom_by(0.8);
                    }
                    if icon_button(ui, Icon::Actual, &tr("t.zoom_100"), at_actual && (!self.fit || !self.fit_up), has_img).clicked() {
                        self.zoom_actual();
                    }
                    if icon_button(ui, Icon::Fit, &tr("m.fit"), self.fit && !(at_actual && !self.fit_up), has_img).clicked() {
                        self.fit = true;
                        self.fit_up = true;
                        self.pan = Vec2::ZERO;
                    }
                    // Niveau de zoom actuel.
                    let zoom_txt = if has_img { format!("{:.0} %", self.zoom * 100.0) } else { String::new() };
                    ui.add_sized([56.0, 24.0], egui::Label::new(egui::RichText::new(zoom_txt).strong()));
                    ui.separator();
                    ui.checkbox(&mut self.settings.show_props, tr("properties"));
                });
                group_right = group.response.rect.right();
                let w = group.response.rect.width();
                if (w - self.toolbar_w).abs() > 0.5 {
                    self.toolbar_w = w;
                    ctx.request_repaint();
                }
            })
            .response
            .rect;
        // Informations à droite de la barre.
        if let Some(s) = &self.shown {
            let f = &s.loaded.pic.frames[s.loaded.pic.frame_index(s.dir, s.frame)];
            let mut info = String::new();
            if let Some(i) = self.index {
                info += &format!("{} / {}   ", i + 1, self.files.len());
            }
            info += &format!("{} × {}", f.width, f.height);
            let galley = ui.painter().layout_no_wrap(info, FontId::proportional(13.0), ui.visuals().weak_text_color());
            // Seulement s'il y a la place à droite du groupe de boutons.
            if bar.right() - 8.0 - galley.size().x > group_right + 12.0 {
                ui.painter().galley(pos2(bar.right() - 8.0 - galley.size().x, bar.center().y - galley.size().y / 2.0), galley, Color32::PLACEHOLDER);
            }
        }
        ui.add_space(3.0);
    }
}
