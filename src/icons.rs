//! Boutons à icônes dessinées au pinceau (indépendants des polices).

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Prev,
    Next,
    ZoomIn,
    ZoomOut,
    Actual,
    Fit,
    Play,
    Pause,
    Folder,
    CopyText,
    CopyJson,
}

pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, selected: bool, enabled: bool) -> Response {
    let size = vec2(32.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let visuals = ui.visuals();
    let style = if !enabled {
        &visuals.widgets.noninteractive
    } else if resp.is_pointer_button_down_on() {
        &visuals.widgets.active
    } else if resp.hovered() {
        &visuals.widgets.hovered
    } else {
        &visuals.widgets.inactive
    };
    let mut fg = visuals.widgets.inactive.fg_stroke.color;
    if !enabled {
        fg = visuals.widgets.noninteractive.fg_stroke.color.gamma_multiply(0.45);
    }
    if selected || resp.hovered() || resp.is_pointer_button_down_on() {
        let fill = if selected { visuals.selection.bg_fill } else { style.bg_fill };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if selected {
        fg = visuals.selection.stroke.color;
    }
    paint_icon(ui, rect.shrink2(vec2(6.0, 4.0)), icon, fg);
    resp.on_hover_text(tip)
}

fn paint_icon(ui: &Ui, r: Rect, icon: Icon, c: Color32) {
    let p = ui.painter();
    let st = Stroke::new(1.6, c);
    let cen = r.center();
    match icon {
        Icon::Prev => tri(p, cen, 7.0, true, c),
        Icon::Next => tri(p, cen, 7.0, false, c),
        Icon::ZoomIn | Icon::ZoomOut => {
            let lc = cen - vec2(2.0, 2.0);
            p.circle_stroke(lc, 6.0, st);
            p.line_segment([lc + vec2(4.5, 4.5), lc + vec2(9.0, 9.0)], Stroke::new(2.2, c));
            p.line_segment([lc - vec2(3.0, 0.0), lc + vec2(3.0, 0.0)], st);
            if matches!(icon, Icon::ZoomIn) {
                p.line_segment([lc - vec2(0.0, 3.0), lc + vec2(0.0, 3.0)], st);
            }
        }
        Icon::Actual => {
            p.text(cen, egui::Align2::CENTER_CENTER, "1:1", egui::FontId::proportional(13.0), c);
        }
        Icon::Fit => {
            let q = r.shrink(2.0);
            for (a, b, d1, d2) in [
                (q.left_top(), q.left_top(), vec2(5.0, 0.0), vec2(0.0, 5.0)),
                (q.right_top(), q.right_top(), vec2(-5.0, 0.0), vec2(0.0, 5.0)),
                (q.left_bottom(), q.left_bottom(), vec2(5.0, 0.0), vec2(0.0, -5.0)),
                (q.right_bottom(), q.right_bottom(), vec2(-5.0, 0.0), vec2(0.0, -5.0)),
            ] {
                p.line_segment([a, a + d1], st);
                p.line_segment([b, b + d2], st);
            }
        }
        Icon::Play => tri(p, cen, 7.0, false, c),
        Icon::CopyText => {
            // Page de texte : feuille avec quelques lignes.
            let q = Rect::from_center_size(cen, vec2(13.0, 16.0));
            p.rect_stroke(q, 1.5, st, egui::StrokeKind::Inside);
            for i in 0..3 {
                let y = q.top() + 4.5 + i as f32 * 3.6;
                p.line_segment([pos2(q.left() + 3.0, y), pos2(q.right() - 3.0, y)], Stroke::new(1.2, c));
            }
        }
        Icon::CopyJson => {
            p.text(cen, egui::Align2::CENTER_CENTER, "{ }", egui::FontId::monospace(14.0), c);
        }
        Icon::Folder => {
            let q = Rect::from_center_size(cen, vec2(18.0, 13.0));
            let tab = Rect::from_min_size(q.left_top() - vec2(0.0, 2.0), vec2(8.0, 3.0));
            p.rect_stroke(q, 1.5, st, egui::StrokeKind::Inside);
            p.rect_filled(tab, 1.0, c);
        }
        Icon::Pause => {
            p.rect_filled(Rect::from_center_size(cen - vec2(3.5, 0.0), vec2(4.0, 13.0)), 1.0, c);
            p.rect_filled(Rect::from_center_size(cen + vec2(3.5, 0.0), vec2(4.0, 13.0)), 1.0, c);
        }
    }
}

fn tri(p: &egui::Painter, cen: Pos2, s: f32, left: bool, c: Color32) {
    let d = if left { -1.0 } else { 1.0 };
    let pts = vec![
        pos2(cen.x + d * s * 0.6, cen.y),
        pos2(cen.x - d * s * 0.5, cen.y - s),
        pos2(cen.x - d * s * 0.5, cen.y + s),
    ];
    p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    let _ = Vec2::ZERO;
}
