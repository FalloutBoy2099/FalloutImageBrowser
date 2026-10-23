//! Fenêtre indépendante et petits widgets partagés.

use crate::model::*;
use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, Sense, Stroke, Ui, pos2, vec2};

/// Fenêtre indépendante (non modale) : fenêtre flottante dans la fenêtre principale ou, avec `native`, vraie fenêtre du système
/// (donc dans la barre des tâches). `open` passe à faux quand l'utilisateur la ferme.
pub fn extra_window(ctx: &egui::Context, native: bool, id: &str, title: String, size: [f32; 2], resizable: bool, centered: bool, open: &mut bool, mut add: impl FnMut(&mut Ui)) {
    if !native {
        let mut w = egui::Window::new(title).id(Id::new(id)).open(open).frame(crate::dialog::window_frame(ctx)).collapsible(false).resizable(resizable).default_size(size);
        if centered {
            w = w.anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0));
        }
        w.show(ctx, |ui| add(ui));
        return;
    }
    let builder =
        egui::ViewportBuilder::default().with_title(title).with_inner_size(size).with_min_inner_size([320.0, 200.0]).with_resizable(resizable).with_maximize_button(resizable);
    let mut closed = false;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(id), builder, |ui, _class| {
        let frame = egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin::same(16));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if resizable {
                add(ui);
            } else {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| add(ui));
            }
        });
        closed |= ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape));
    });
    if closed {
        *open = false;
    }
}

/// Première lettre ou chiffre d'un nom (en majuscule), pour le bouton d'un favori.
pub(super) fn letter_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).or_else(|| name.chars().next()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_string())
}

/// Bouton carré affichant une lettre.
pub(super) fn letter_button(ui: &mut Ui, letter: &str, tip: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let v = ui.visuals();
    let down = resp.is_pointer_button_down_on();
    let fill = if selected {
        v.selection.bg_fill
    } else if down {
        v.widgets.active.bg_fill
    } else if resp.hovered() {
        v.widgets.hovered.bg_fill
    } else {
        v.widgets.inactive.weak_bg_fill
    };
    let fg = if selected { v.selection.stroke.color } else { v.widgets.inactive.fg_stroke.color };
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, FontId::proportional(15.0), fg);
    resp.on_hover_text(tip)
}

/// Curseur à dégradé : `value` entre 0 et 1, `track(t)` donne la couleur du dégradé en `t`. Renvoie vrai si la valeur change.
pub(super) fn gradient_slider(ui: &mut Ui, value: &mut f32, width: f32, track: impl Fn(f32) -> Color32) -> bool {
    const HANDLE: f32 = 9.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    // La barre est rentrée du rayon de la poignée : la poignée ne dépasse jamais de la zone du curseur.
    let bar = Rect::from_min_max(pos2(rect.left() + HANDLE, rect.center().y - 5.5), pos2(rect.right() - HANDLE, rect.center().y + 5.5));
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        *value = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        changed = true;
    }
    let n = 48u32;
    let mut mesh = egui::Mesh::default();
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = bar.left() + t * bar.width();
        let col = track(t);
        mesh.colored_vertex(pos2(x, bar.top()), col);
        mesh.colored_vertex(pos2(x, bar.bottom()), col);
        if i > 0 {
            let k = i * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(bar, 5.0, Stroke::new(1.0, Color32::from_white_alpha(45)), egui::StrokeKind::Outside);
    let c = pos2(bar.left() + *value * bar.width(), bar.center().y);
    ui.painter().circle_filled(c, HANDLE, track(*value));
    ui.painter().circle_stroke(c, HANDLE, Stroke::new(2.5, Color32::WHITE));
    changed
}

/// Grille de la palette ; renvoie la couleur cliquée avec Alt (`true` = clic droit).
pub(super) fn palette_grid(ui: &mut Ui, pal: &Palette) -> Option<(bool, [u8; 4])> {
    let cell = 12.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(cell * 16.0, cell * 16.0), Sense::click());
    let painter = ui.painter_at(rect);
    for i in 0..256usize {
        let (x, y) = (i % 16, i / 16);
        let r = Rect::from_min_size(rect.min + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell));
        let c = pal.colors[i];
        if pal.used[i] && c[3] > 0 {
            painter.rect_filled(r, 0.0, Color32::from_rgb(c[0], c[1], c[2]));
        } else {
            painter.rect_filled(r, 0.0, Color32::from_gray(40));
            painter.line_segment([r.left_top(), r.right_bottom()], Stroke::new(1.0, Color32::from_gray(90)));
        }
    }
    let alt = ui.input(|i| i.modifiers.alt);
    let (left, right) = (alt && resp.clicked(), alt && resp.secondary_clicked());
    let mut picked = None;
    if let Some(p) = resp.hover_pos() {
        let (x, y) = (((p.x - rect.left()) / cell) as usize, ((p.y - rect.top()) / cell) as usize);
        if x < 16 && y < 16 {
            let i = y * 16 + x;
            let c = pal.colors[i];
            if left || right {
                picked = Some((right, if pal.used[i] { c } else { [0, 0, 0, 255] }));
            }
            resp.on_hover_text(format!(
                "#{i}   RGB {}, {}, {}   #{:02X}{:02X}{:02X}{}",
                c[0],
                c[1],
                c[2],
                c[0],
                c[1],
                c[2],
                if !pal.used[i] {
                    "  (unused)"
                } else if c[3] == 0 {
                    "  (transparent)"
                } else {
                    ""
                }
            ));
        }
    }
    picked
}
