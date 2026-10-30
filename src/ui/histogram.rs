//! Histogramme de l'image : canaux R, V, B et luminance, échelle réglable, écrêtage et lecture sous le curseur.

use crate::app::{HistoScale, HistoSettings, HistoStyle};
use crate::i18n::{tr, trf};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

/// Comptes par niveau : rouge, vert, bleu, luminance.
pub type Histogram = [[u32; 256]; 4];

const CHANNELS: [(&str, [u8; 3]); 4] = [("histo.r", [225, 45, 45]), ("histo.g", [45, 190, 60]), ("histo.b", [55, 105, 240]), ("histo.l", [200, 200, 200])];

/// Histogramme d'une image RGBA ; les pixels transparents sont ignorés.
pub fn compute(rgba: &[u8]) -> Histogram {
    let mut h = [[0u32; 256]; 4];
    for p in rgba.chunks_exact(4) {
        if p[3] == 0 {
            continue;
        }
        let (r, g, b) = (p[0] as u32, p[1] as u32, p[2] as u32);
        h[0][r as usize] += 1;
        h[1][g as usize] += 1;
        h[2][b as usize] += 1;
        h[3][((r * 2126 + g * 7152 + b * 722) / 10000) as usize] += 1;
    }
    h
}

fn scaled(v: f32, max: f32, scale: HistoScale) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let t = match scale {
        HistoScale::Linear => v / max,
        HistoScale::Sqrt => (v / max).sqrt(),
        HistoScale::Log => (1.0 + v).ln() / (1.0 + max).ln(),
    };
    t.clamp(0.0, 1.0)
}

/// Réglages (canaux, échelle, remplissage, grille) puis le graphique.
pub fn show(ui: &mut Ui, h: &Histogram, set: &mut HistoSettings) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (key, col)) in CHANNELS.iter().enumerate() {
            let on = &mut set.channels[i];
            let text = egui::RichText::new(tr(key)).strong().color(if *on { Color32::from_rgb(col[0], col[1], col[2]) } else { ui.visuals().weak_text_color() });
            if ui.add(egui::Button::selectable(*on, text).min_size(vec2(26.0, 20.0))).clicked() {
                *on = !*on;
            }
        }
        ui.separator();
        egui::ComboBox::from_id_salt("histo_scale")
            .width(90.0)
            .selected_text(tr(set.scale.key()))
            .show_ui(ui, |ui| {
                for s in [HistoScale::Linear, HistoScale::Sqrt, HistoScale::Log] {
                    ui.selectable_value(&mut set.scale, s, tr(s.key()));
                }
            })
            .response
            .on_hover_text(tr("histo.scale"));
    });
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("histo_style").width(80.0).selected_text(tr(set.style.key())).show_ui(ui, |ui| {
            for s in [HistoStyle::Bars, HistoStyle::Curve] {
                ui.selectable_value(&mut set.style, s, tr(s.key()));
            }
        });
        ui.checkbox(&mut set.grid, tr("histo.grid"));
        ui.checkbox(&mut set.clip, tr("histo.clip"));
    });
    draw(ui, h, set);
}

/// Couleur d'une zone selon les canaux qui la recouvrent (bit 0 : rouge, 1 : vert, 2 : bleu), comme Lightroom / Camera Raw :
/// un canal seul garde sa couleur, deux canaux donnent jaune, cyan ou magenta, les trois donnent le gris.
fn mix(mask: u8, gray: u8) -> Color32 {
    match mask {
        0b001 => Color32::from_rgb(225, 50, 50),
        0b010 => Color32::from_rgb(55, 185, 60),
        0b100 => Color32::from_rgb(55, 100, 235),
        0b011 => Color32::from_rgb(235, 215, 45),
        0b110 => Color32::from_rgb(45, 200, 215),
        0b101 => Color32::from_rgb(215, 45, 205),
        _ => Color32::from_gray(gray),
    }
}

/// Lissage léger (noyau 1-2-3-2-1) : les courbes d'une image à peu de couleurs (palette) seraient sinon en pics.
fn smooth(src: &[u32; 256]) -> [f32; 256] {
    const K: [f32; 5] = [1.0, 2.0, 3.0, 2.0, 1.0];
    let mut out = [0f32; 256];
    for (i, o) in out.iter_mut().enumerate() {
        let (mut acc, mut wsum) = (0.0, 0.0);
        for (k, wk) in K.iter().enumerate() {
            let j = i as i32 + k as i32 - 2;
            if (0..256).contains(&j) {
                acc += src[j as usize] as f32 * wk;
                wsum += wk;
            }
        }
        *o = acc / wsum;
    }
    out
}

fn draw(ui: &mut Ui, h: &Histogram, set: &HistoSettings) {
    // Marge à droite : le cadre s'arrête avant la barre de défilement.
    let w = (ui.available_width() - 16.0).clamp(128.0, 560.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 120.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::from_gray(38));
    let plot = rect.shrink2(vec2(1.0, 2.0));
    let total = h[3].iter().map(|&v| v as u64).sum::<u64>().max(1) as f32;
    let on: Vec<usize> = (0..4).filter(|&i| set.channels[i]).collect();
    // Courbe : comptes lissés ; barres : comptes bruts.
    let curve = set.style == HistoStyle::Curve;
    let hs: Vec<[f32; 256]> = (0..4).map(|c| if curve { smooth(&h[c]) } else { h[c].map(|v| v as f32) }).collect();
    // Échelle : plus grand compte hors extrémités (0 et 255 écrasent tout : noir, blanc, fond).
    let max = on.iter().flat_map(|&c| hs[c][1..255].iter()).copied().fold(0.0f32, f32::max).max(1.0);
    if set.grid {
        let grid = Stroke::new(1.0, Color32::from_white_alpha(14));
        for i in 1..8 {
            let x = plot.left() + plot.width() * i as f32 / 8.0;
            p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], grid);
        }
        for i in 1..4 {
            let y = plot.top() + plot.height() * i as f32 / 4.0;
            p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], grid);
        }
    }
    // Une colonne par pixel d'écran (maximum des niveaux qu'elle couvre).
    let cols = plot.width().floor().max(1.0) as usize;
    let height_at = |c: usize, col: usize| -> f32 {
        let lo = col * 256 / cols;
        let hi = ((col + 1) * 256 / cols).max(lo + 1).min(256);
        let v = hs[c][lo..hi].iter().copied().fold(0.0f32, f32::max);
        scaled(v, max, set.scale) * plot.height()
    };
    let colw = plot.width() / cols as f32;
    let rgb_on: Vec<usize> = on.iter().copied().filter(|&c| c < 3).collect();
    for col in 0..cols {
        let t = col as f32 / cols as f32;
        let gray = (150.0 + 95.0 * t) as u8; // dégradé gris foncé → blanc, comme Lightroom
        let x0 = plot.left() + col as f32 * colw;
        // Barres : un trait vide entre deux niveaux quand il y a la place (une colonne ≥ 3 px).
        let x1 = if curve { x0 + colw + 0.5 } else { x0 + (colw - if colw >= 3.0 { 1.0 } else { 0.0 }).max(1.0) };
        let seg = |lo: f32, hi: f32, color: Color32| p.rect_filled(Rect::from_min_max(pos2(x0, plot.bottom() - hi), pos2(x1, plot.bottom() - lo)), 0.0, color);
        // Luminance : zone grise sous les canaux (seule visible quand aucun canal couleur n'est affiché).
        if set.channels[3] {
            let hh = height_at(3, col);
            seg(0.0, hh, Color32::from_gray(if rgb_on.is_empty() { gray } else { 120 }));
        }
        // Canaux : du plus bas au plus haut ; chaque tranche prend la couleur des canaux qui l'atteignent.
        let mut levels: Vec<(f32, usize)> = rgb_on.iter().map(|&c| (height_at(c, col), c)).collect();
        levels.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut base = 0.0;
        for k in 0..levels.len() {
            let top = levels[k].0;
            if top > base {
                let mask = levels[k..].iter().fold(0u8, |m, &(_, ch)| m | 1 << ch);
                seg(base, top, mix(mask, gray));
                base = top;
            }
        }
    }
    // Contour fin de la forme (courbe) : les crêtes ressortent sur le remplissage.
    if curve && let Some(&top_ch) = on.iter().find(|&&c| c == 3).or(on.first()) {
        let pts: Vec<Pos2> = (0..cols).map(|col| pos2(plot.left() + (col as f32 + 0.5) * colw, plot.bottom() - height_at(top_ch, col))).collect();
        p.add(Shape::line(pts, Stroke::new(1.0, Color32::from_white_alpha(70))));
    }
    p.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(10)), egui::StrokeKind::Inside);
    // Écrêtage : triangles aux coins ; allumés dans la couleur des canaux écrêtés (blanc : les trois ; magenta : rouge et bleu...).
    let mut tip: Option<String> = None;
    if set.clip {
        for (lvl, left) in [(0usize, true), (255usize, false)] {
            let n = on.iter().map(|&c| h[c][lvl]).max().unwrap_or(0);
            let mask = rgb_on.iter().filter(|&&c| h[c][lvl] > 0).fold(0u8, |m, &c| m | 1 << c);
            let boxr = Rect::from_min_size(if left { rect.left_top() + vec2(4.0, 4.0) } else { rect.right_top() + vec2(-20.0, 4.0) }, vec2(16.0, 16.0));
            p.rect_filled(boxr, 2.0, Color32::from_gray(26));
            p.rect_stroke(boxr, 2.0, Stroke::new(1.0, Color32::from_gray(150)), egui::StrokeKind::Inside);
            let col = if n == 0 {
                Color32::from_gray(80)
            } else if mask == 0 {
                Color32::from_gray(230)
            } else {
                mix(mask, 245)
            };
            let r = boxr.shrink2(vec2(3.0, 3.5));
            p.add(Shape::convex_polygon(vec![r.center_top(), r.left_bottom(), r.right_bottom()], col, Stroke::NONE));
            if resp.hover_pos().is_some_and(|q| boxr.expand(2.0).contains(q)) {
                let key = if left { "histo.shadows" } else { "histo.highlights" };
                tip = Some(trf(key, &[("pct", format!("{:.2}", n as f32 / total * 100.0))]));
            }
        }
    }
    // Lecture sous le curseur : niveau et part des pixels de chaque canal.
    let hover = resp.hover_pos().filter(|q| plot.contains(*q)).map(|q| (((q.x - plot.left()) / plot.width() * 256.0) as usize).min(255));
    if let Some(lvl) = hover {
        let x = plot.left() + (lvl as f32 + 0.5) / 256.0 * plot.width();
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, Color32::from_white_alpha(90)));
    }
    if let Some(t) = tip {
        resp.on_hover_text(t);
    }
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let small = |t: String, c: Color32| egui::RichText::new(t).monospace().size(11.0).color(c);
        match hover {
            Some(lvl) => {
                ui.label(small(format!("{} {lvl:>3}", tr("histo.level")), ui.visuals().strong_text_color()));
                for (c, (key, col)) in CHANNELS.iter().enumerate().take(3) {
                    ui.label(small(format!("{} {:.2}%", tr(key), h[c][lvl] as f32 / total * 100.0), Color32::from_rgb(col[0], col[1], col[2])));
                }
            }
            None => {
                // Niveau moyen de chaque canal, en pourcentage (0 à 100).
                for (c, (key, col)) in CHANNELS.iter().enumerate().take(3) {
                    let mean = h[c].iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum::<f64>() / total as f64 / 255.0 * 100.0;
                    ui.label(small(format!("{} {mean:.1}", tr(key)), Color32::from_rgb(col[0], col[1], col[2])));
                }
                ui.label(small(tr("histo.mean"), ui.visuals().weak_text_color()));
            }
        }
    });
}
