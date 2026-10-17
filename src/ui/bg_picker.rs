//! Sélecteur de couleur du fond personnalisé.

use crate::app::App;
use crate::i18n::tr;
use crate::view::{Background, hex_rgb, hsv_to_rgb, parse_hex, push_recent, rgb_to_hsv};
use eframe::egui::{self, Color32, FontId, Id, Rect, Sense, Stroke, pos2, vec2};

use super::widgets::gradient_slider;

impl App {
    /// Ouvre le sélecteur de couleur du fond personnalisé (fenêtre modale).
    pub fn open_bg_picker(&mut self) {
        self.bg_picker_open = true;
        self.bg_picker_prev = (self.settings.background, self.settings.bg_custom, self.settings.bg_alpha);
        self.bg_hsv = rgb_to_hsv(self.settings.bg_custom);
        self.bg_hex = hex_rgb(self.settings.bg_custom);
    }

    /// Ferme le sélecteur en gardant la couleur choisie, qui rejoint les couleurs récentes.
    pub fn close_bg_picker(&mut self) {
        if self.bg_picker_open {
            self.bg_picker_open = false;
            push_recent(&mut self.settings.bg_recent, self.settings.bg_custom, 12);
        }
    }

    /// Fenêtre modale du fond personnalisé : aperçu, valeur hexadécimale, curseurs teinte / saturation / valeur et couleurs
    /// récentes. Le fond affiché derrière change en direct ; Annuler, Échap ou un clic en dehors rétablissent l'ancien fond.
    pub fn bg_picker_window(&mut self, ctx: &egui::Context) {
        if !self.bg_picker_open {
            return;
        }
        const PRESETS: [[u8; 3]; 12] = [
            [0, 0, 0],
            [32, 32, 32],
            [64, 64, 64],
            [128, 128, 128],
            [200, 200, 200],
            [255, 255, 255],
            [30, 50, 30],
            [60, 45, 25],
            [25, 40, 70],
            [70, 25, 30],
            [212, 175, 55],
            [120, 160, 200],
        ];
        // Tant que la fenêtre est ouverte, le fond affiché est le fond personnalisé.
        self.settings.background = Background::Custom;
        let (mut ok, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("bg_picker")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(430.0);
            crate::dialog::setup(ui);
            cancel |= crate::dialog::title(ui, &tr("m.background"));
            ui.add_space(4.0);
            let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
            // Aperçu de la couleur.
            let (preview, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
            let ck = self.checker_id(ctx);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(preview.width() * ctx.pixels_per_point() / 16.0, preview.height() * ctx.pixels_per_point() / 16.0));
            ui.painter().add(egui::epaint::RectShape::filled(preview, 8.0, Color32::WHITE).with_texture(ck, uv));
            ui.painter().rect_filled(
                preview,
                8.0,
                Color32::from_rgba_unmultiplied(self.settings.bg_custom[0], self.settings.bg_custom[1], self.settings.bg_custom[2], self.settings.bg_alpha),
            );
            ui.painter().rect_stroke(preview, 8.0, Stroke::new(1.0, Color32::from_white_alpha(45)), egui::StrokeKind::Inside);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("HEX").weak());
                let edit = ui.add(egui::TextEdit::singleline(&mut self.bg_hex).font(FontId::monospace(14.0)).desired_width(110.0).margin(vec2(8.0, 6.0)));
                if edit.changed()
                    && let Some(c) = parse_hex(&self.bg_hex)
                {
                    self.settings.bg_custom = c;
                    self.bg_hsv = rgb_to_hsv(c);
                }
            });
            let slider_w = ui.available_width() - 52.0;
            let (mut h, mut sat, mut val) = self.bg_hsv;
            let mut changed = false;
            ui.horizontal(|ui| {
                changed |= gradient_slider(ui, &mut h, slider_w, |t| rgb(hsv_to_rgb(t, 1.0, 1.0)));
                ui.label(format!("{:>3}", (h * 360.0).round() as i32));
            });
            ui.horizontal(|ui| {
                let (hh, vv) = (h, val);
                changed |= gradient_slider(ui, &mut sat, slider_w, |t| rgb(hsv_to_rgb(hh, t, vv)));
                ui.label(format!("{:>3}", (sat * 100.0).round() as i32));
            });
            ui.horizontal(|ui| {
                let (hh, ss) = (h, sat);
                changed |= gradient_slider(ui, &mut val, slider_w, |t| rgb(hsv_to_rgb(hh, ss, t)));
                ui.label(format!("{:>3}", (val * 100.0).round() as i32));
            });
            // Opacité : du transparent (damier visible) à la couleur pleine.
            ui.horizontal(|ui| {
                let mut a = self.settings.bg_alpha as f32 / 255.0;
                let [r, g, b] = self.settings.bg_custom;
                if gradient_slider(ui, &mut a, slider_w, |t| Color32::from_rgba_unmultiplied(r, g, b, (t * 255.0).round() as u8)) {
                    self.settings.bg_alpha = (a * 255.0).round() as u8;
                }
                ui.label(format!("{:>3}", (self.settings.bg_alpha as f32 / 255.0 * 100.0).round() as i32));
            });
            if changed {
                self.bg_hsv = (h, sat, val);
                self.settings.bg_custom = hsv_to_rgb(h, sat, val);
                self.bg_hex = hex_rgb(self.settings.bg_custom);
            }
            // Couleurs récentes, complétées par quelques couleurs usuelles.
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tr("m.bg_recent")).weak());
            let mut swatches = self.settings.bg_recent.clone();
            for p in PRESETS {
                if swatches.len() < 12 && !swatches.contains(&p) {
                    swatches.push(p);
                }
            }
            let mut pick = None;
            egui::Grid::new("bg_recent").spacing([8.0, 8.0]).show(ui, |ui| {
                for (i, c) in swatches.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    ui.painter().rect_filled(r, 9.0, rgb(*c));
                    let stroke = if *c == self.settings.bg_custom {
                        Stroke::new(2.0, Color32::WHITE)
                    } else if resp.hovered() {
                        Stroke::new(1.5, Color32::from_white_alpha(140))
                    } else {
                        Stroke::new(1.0, Color32::from_white_alpha(40))
                    };
                    ui.painter().rect_stroke(r, 9.0, stroke, egui::StrokeKind::Inside);
                    if resp.clicked() {
                        pick = Some(*c);
                    }
                    if i % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
            if let Some(c) = pick {
                self.settings.bg_custom = c;
                self.bg_hsv = rgb_to_hsv(c);
                self.bg_hex = hex_rgb(c);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ok = crate::dialog::button(ui, tr("c.ok")).clicked();
                cancel |= crate::dialog::button(ui, tr("c.cancel")).clicked();
            });
        });
        if ok {
            self.close_bg_picker();
        } else if cancel || modal.should_close() {
            (self.settings.background, self.settings.bg_custom, self.settings.bg_alpha) = self.bg_picker_prev;
            self.bg_picker_open = false;
        }
    }
}
