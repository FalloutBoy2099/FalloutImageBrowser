//! Zone d'affichage de l'image : textures, zoom, déplacement et barre d'animation.

use crate::app::{App, OriginStyle, Smoothing, TexEntry};
use crate::i18n::tr;
use crate::icons::{Icon, icon_button};
use crate::model::*;
use crate::view::Background;
use crate::view::{anchor_in_canvas, box_downscale, can_show_all_dirs, canvas_size, dir_grid, hex_color, pixel_info, render_all_dirs, render_rgba_cow};
use eframe::egui::{self, Align2, Color32, ColorImage, FontId, Id, Rect, Sense, Stroke, TextureOptions, Ui, Vec2, pos2, vec2};
use rayon::prelude::*;
use std::borrow::Cow;
use std::time::Duration;

use super::{MAX_ZOOM, MIN_ZOOM};

/// Mémoire graphique maximale des textures de l'image affichée (toutes les images / niveaux / transformations confondus).
const TEXTURE_BUDGET: usize = 256 << 20;

/// Image egui à partir de pixels RGBA non prémultipliés (conversion en parallèle pour les grandes images).
fn to_color_image(rgba: &[u8], w: usize, h: usize) -> ColorImage {
    let (w, h) = (w.max(1), h.max(1));
    let mut pixels = vec![Color32::TRANSPARENT; w * h];
    if rgba.len() >= w * h * 4 {
        pixels.par_chunks_mut(w).zip(rgba.par_chunks(w * 4)).for_each(|(row, src)| {
            for (d, p) in row.iter_mut().zip(src.chunks_exact(4)) {
                *d = Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
            }
        });
    }
    ColorImage { size: [w, h], source_size: Vec2::new(w as f32, h as f32), pixels }
}

impl App {
    pub(super) fn checker_id(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.checker.is_none() {
            let (a, b) = (Color32::from_gray(0x70), Color32::from_gray(0x88));
            let mut px = Vec::with_capacity(256);
            for y in 0..16 {
                for x in 0..16 {
                    px.push(if (x / 8 + y / 8) % 2 == 0 { a } else { b });
                }
            }
            let img = ColorImage { size: [16, 16], source_size: Vec2::new(16.0, 16.0), pixels: px };
            let opts = TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Nearest,
                wrap_mode: egui::TextureWrapMode::Repeat,
                mipmap_mode: None,
            };
            self.checker = Some(ctx.load_texture("checker", img, opts));
        }
        self.checker.as_ref().unwrap().id()
    }

    /// Texture de l'image `frame` de la direction affichée, avec la rotation / les symétries de la vue ; renvoie aussi les
    /// dimensions (après transformation) de l'image complète.
    fn ensure_tex(&mut self, ctx: &egui::Context, level: u8, frame: usize) -> Option<(egui::TextureId, u32, u32)> {
        let smoothing = self.settings.smoothing;
        let transform = self.transform;
        let all = self.settings.show_all_dirs && self.shown.as_ref().is_some_and(|s| can_show_all_dirs(&s.loaded.pic));
        let s = self.shown.as_mut()?;
        let pic = &s.loaded.pic;
        // Toutes les directions côte à côte : une seule texture, repérée par une « direction » spéciale.
        let dir_key = if all { usize::MAX } else { s.dir };
        let (cw0, ch0) = if all { dir_grid(pic).size() } else { canvas_size(pic, s.dir) };
        let (w, h) = transform.size(cw0 as usize, ch0 as usize);
        let (w, h) = (w as u32, h as u32);
        let key = (dir_key, frame, level, transform.id());
        let pass = ctx.cumulative_pass_nr();
        if let Some(e) = s.tex.get_mut(&key) {
            e.used = pass;
        } else {
            let cap = if pic.is_animated() { 96 } else { 8 };
            let k = 1usize << level;
            let (cw, ch, rgba) = if all {
                let (w, h, v) = render_all_dirs(pic, frame);
                (w, h, Cow::Owned(v))
            } else {
                render_rgba_cow(pic, s.dir, frame)
            };
            let (rgba, cw, ch) = transform.apply_cow(rgba, cw, ch);
            let (rgba, tw, th) = if k > 1 {
                let (v, tw, th) = box_downscale(&rgba, cw, ch, k);
                (Cow::Owned(v), tw, th)
            } else {
                (rgba, cw, ch)
            };
            let img = to_color_image(&rgba, tw, th);
            // Éviction des textures les moins récemment utilisées : par nombre, et par taille en mémoire graphique. Une texture
            // demandée pendant ce passage n'est jamais retirée (elle sera peinte dans ce même passage).
            let bytes = img.size[0] * img.size[1] * 4;
            while s.tex.len() >= cap || (!s.tex.is_empty() && s.tex.values().map(|e| e.bytes).sum::<usize>() + bytes > TEXTURE_BUDGET) {
                let victim = s.tex.iter().filter(|(_, e)| e.used != pass).min_by_key(|(_, e)| e.used).map(|(k, _)| *k);
                match victim {
                    Some(v) => {
                        s.tex.remove(&v);
                    }
                    None => break,
                }
            }
            let pixelated = matches!(pic.frames[0].pixels, Pixels::Indexed(_)) || w.max(h) <= 512;
            let mag = match smoothing {
                Smoothing::Auto if pixelated => egui::TextureFilter::Nearest,
                Smoothing::Off => egui::TextureFilter::Nearest,
                _ => egui::TextureFilter::Linear,
            };
            let min = if smoothing == Smoothing::Off { egui::TextureFilter::Nearest } else { egui::TextureFilter::Linear };
            let handle = ctx.load_texture(
                format!("img-{}-{}-{}-{}", dir_key, frame, level, transform.id()),
                img,
                TextureOptions { magnification: mag, minification: min, wrap_mode: egui::TextureWrapMode::ClampToEdge, mipmap_mode: None },
            );
            s.tex.insert(key, TexEntry { handle, bytes, used: pass });
        }
        Some((s.tex[&key].handle.id(), w, h))
    }

    pub fn canvas(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        let painter = ui.painter_at(rect);
        // Fond de toute la zone d'affichage : couleur choisie, sinon celle du thème (le damier n'est dessiné que derrière l'image).
        let translucent = self.settings.background == Background::Custom && self.settings.bg_alpha < 255;
        let backdrop = match self.settings.background.color(self.settings.bg_custom) {
            Some(c) if !translucent => Color32::from_rgb(c[0], c[1], c[2]),
            _ => ui.visuals().extreme_bg_color,
        };
        painter.rect_filled(rect, 0.0, backdrop);
        // Textes posés sur le fond : clairs sur un fond sombre, sombres sur un fond clair (quel que soit le thème de l'interface).
        let dark_backdrop = (backdrop.r() as u32 * 299 + backdrop.g() as u32 * 587 + backdrop.b() as u32 * 114) / 1000 < 128;
        let (text_strong, text_weak) = if dark_backdrop { (Color32::from_gray(235), Color32::from_gray(165)) } else { (Color32::from_gray(20), Color32::from_gray(90)) };
        let mut nav_step = 0i32;
        // Alt enfoncé : l'image est bloquée (pas de déplacement, zoom, défilement ni double-clic) ; Alt + clic prélève les couleurs.
        let locked = ctx.input(|i| i.modifiers.alt);

        if let Some(s) = &self.shown {
            // Toutes les directions côte à côte (FRM, ART...) ou la direction choisie seule.
            let grid = (self.settings.show_all_dirs && can_show_all_dirs(&s.loaded.pic)).then(|| dir_grid(&s.loaded.pic));
            let (cw0, ch0) = match &grid {
                Some(g) => g.size(),
                None => canvas_size(&s.loaded.pic, s.dir),
            };
            let (iw, ih) = self.transform.size(cw0 as usize, ch0 as usize);
            let (iw, ih) = (iw as f32, ih as f32);
            // Zoom « ajusté ».
            if self.fit {
                let z = (rect.width() * ppp / iw).min(rect.height() * ppp / ih);
                // Au démarrage et à l'ouverture, une petite image n'est pas agrandie ; la commande « Ajuster à la fenêtre » l'agrandit.
                let z = if self.fit_up { z } else { z.min(1.0) };
                self.zoom = z.clamp(MIN_ZOOM, MAX_ZOOM);
                self.pan = Vec2::ZERO;
            }
            // Interactions.
            if !locked && resp.dragged() {
                if self.fit {
                    self.fit = false;
                }
                self.pan += resp.drag_delta();
            }
            // Clic de molette (molette en mode zoom) : retour à 100 %.
            if !locked && !self.settings.wheel_nav && resp.hovered() && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Middle)) {
                self.zoom_actual();
            }
            if !locked && resp.double_clicked() {
                if self.fit && (self.zoom - 1.0).abs() > 1e-3 {
                    self.zoom_actual();
                } else if self.fit {
                    // déjà à 100 %
                } else {
                    self.fit = true;
                    self.fit_up = true;
                }
            }
            if !locked && resp.hovered() {
                let nav = self.settings.wheel_nav;
                let ctrl = ctx.input(|i| i.modifiers.command);
                if nav && !ctrl {
                    // Molette = image précédente / suivante.
                    let raw: f32 = ctx.input(|i| i.raw.events.iter().map(|e| if let egui::Event::MouseWheel { delta, .. } = e { delta.y } else { 0.0 }).sum());
                    if raw > 0.0 {
                        nav_step = -1;
                    } else if raw < 0.0 {
                        nav_step = 1;
                    }
                }
                let scroll = if nav && !ctrl { 0.0 } else { ctx.input(|i| i.smooth_scroll_delta.y) };
                let pinch = ctx.input(|i| i.zoom_delta());
                let mut factor = (scroll * 0.0025).exp() * pinch;
                if (factor - 1.0).abs() < 1e-4 {
                    factor = 1.0;
                }
                if factor != 1.0
                    && let Some(cur) = resp.hover_pos()
                {
                    let old = self.zoom;
                    let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                    let ratio = new / old;
                    let rel = cur - rect.center();
                    self.pan = rel - (rel - self.pan) * ratio;
                    self.zoom = new;
                    self.fit = false;
                }
            }
            // Limiter le déplacement.
            let size_pts = vec2(iw, ih) * self.zoom / ppp;
            let lim = vec2(size_pts.x * 0.5 + rect.width() * 0.5 - 40.0, size_pts.y * 0.5 + rect.height() * 0.5 - 40.0).max(Vec2::ZERO);
            self.pan = self.pan.clamp(-lim, lim);

            // Niveau de réduction (pseudo-mipmap) pour les grandes images affichées en petit.
            let mut level = 0u8;
            if self.zoom < 0.5 && iw.max(ih) >= 768.0 {
                level = ((1.0 / self.zoom).log2().floor() as i32).clamp(0, 8) as u8;
            }
            let max_side = ctx.input(|i| i.max_texture_side) as f32;
            while (iw / (1u32 << level) as f32).max(ih / (1u32 << level) as f32) > max_side && level < 12 {
                level += 1;
            }
            let (cur_frame, prev_frame) = match &self.shown {
                Some(s) => {
                    let n = s.loaded.pic.dir_len(s.dir);
                    (s.frame, if n > 1 { Some(if s.frame > 0 { s.frame - 1 } else { n - 1 }) } else { None })
                }
                None => (0, None),
            };
            let tex = self.ensure_tex(&ctx, level, cur_frame);
            // Pelure d'oignon : l'image précédente de l'animation, en transparence, sous l'image courante.
            let onion_tex = if self.settings.onion && grid.is_none() { prev_frame.and_then(|f| self.ensure_tex(&ctx, level, f)) } else { None };
            // Point d'ancrage (centre de l'hexagone pour un FRM), dans les coordonnées de la vue transformée.
            let has_anchor = self.shown.as_ref().is_some_and(|s| matches!(s.loaded.pic.format, Format::Frm | Format::Art | Format::Spr));
            let anchors: Vec<(f32, f32)> = if self.settings.origin != OriginStyle::Off && has_anchor {
                let to_view = |(ax, ay): (i32, i32)| self.transform.map_point(ax as f32, ay as f32, cw0 as usize, ch0 as usize);
                match (&grid, &self.shown) {
                    (Some(g), _) => (0..g.n).map(|d| to_view(g.anchor(d))).collect(),
                    (None, Some(s)) => vec![to_view(anchor_in_canvas(&s.loaded.pic, s.dir))],
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            let mut select_dir: Option<usize> = None;
            let mut pick_color: Option<(bool, [u8; 4])> = None;
            if let Some((tid, w, h)) = tex {
                let sc = self.zoom / ppp; // points par pixel de l'image
                let size_pts = vec2(w as f32, h as f32) * sc;
                let snap = |v: f32| (v * ppp).round() / ppp;
                let mut min = rect.center() + self.pan - size_pts * 0.5;
                min = pos2(snap(min.x), snap(min.y));
                let img_rect = Rect::from_min_size(min, size_pts);
                let vis = img_rect.intersect(rect);
                if vis.is_positive() {
                    let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    // Fond en damier sur toute la zone d'affichage (un fond uni est déjà peint plus haut) ; le damier reste fixe quand on déplace l'image.
                    if self.settings.background.color(self.settings.bg_custom).is_none() || translucent {
                        let ck = self.checker_id(&ctx);
                        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() * ppp / 16.0, rect.height() * ppp / 16.0));
                        painter.image(ck, rect, uv, Color32::WHITE);
                    }
                    // Fond personnalisé translucide : la couleur se superpose au damier.
                    if translucent {
                        let [r, g, b] = self.settings.bg_custom;
                        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(r, g, b, self.settings.bg_alpha));
                    }
                    if let Some((oid, _, _)) = onion_tex {
                        painter.image(oid, img_rect, full_uv, Color32::from_white_alpha(90));
                    }
                    painter.image(tid, img_rect, full_uv, Color32::WHITE);
                    // Grille de pixels (à partir de 4 points par pixel, soit un zoom de 400 %).
                    if self.settings.show_grid && sc >= 4.0 && level == 0 {
                        let stroke = Stroke::new(1.0, Color32::from_black_alpha(90));
                        let (i0, i1) = (((vis.min.x - img_rect.min.x) / sc).floor() as i32, ((vis.max.x - img_rect.min.x) / sc).ceil() as i32);
                        for i in i0..=i1 {
                            let x = img_rect.min.x + i as f32 * sc;
                            if x >= vis.min.x && x <= vis.max.x {
                                painter.line_segment([pos2(x, vis.min.y), pos2(x, vis.max.y)], stroke);
                            }
                        }
                        let (j0, j1) = (((vis.min.y - img_rect.min.y) / sc).floor() as i32, ((vis.max.y - img_rect.min.y) / sc).ceil() as i32);
                        for j in j0..=j1 {
                            let y = img_rect.min.y + j as f32 * sc;
                            if y >= vis.min.y && y <= vis.max.y {
                                painter.line_segment([pos2(vis.min.x, y), pos2(vis.max.x, y)], stroke);
                            }
                        }
                    }
                    // Cellules des directions : cadre, numéro, et mise en évidence de la direction choisie.
                    if let (Some(g), Some(s)) = (&grid, &self.shown) {
                        for d in 0..g.n {
                            let (cx, cy) = g.cell_origin(d);
                            let (p0, p1) = (
                                self.transform.map_point(cx as f32, cy as f32, cw0 as usize, ch0 as usize),
                                self.transform.map_point((cx + g.cell_w) as f32, (cy + g.cell_h) as f32, cw0 as usize, ch0 as usize),
                            );
                            let r = Rect::from_two_pos(img_rect.min + vec2(p0.0, p0.1) * sc, img_rect.min + vec2(p1.0, p1.1) * sc);
                            let chosen = d == s.dir;
                            let col = if chosen { Color32::from_rgb(240, 190, 70) } else { Color32::from_white_alpha(70) };
                            painter.rect_stroke(r, 0.0, Stroke::new(if chosen { 2.0 } else { 1.0 }, col), egui::StrokeKind::Inside);
                            let label = s.loaded.pic.dir_label(d);
                            let font = FontId::proportional(13.0);
                            let galley = painter.layout_no_wrap(label, font, col);
                            let at = r.min + vec2(6.0, 4.0);
                            painter.rect_filled(Rect::from_min_size(at - vec2(3.0, 1.0), galley.size() + vec2(6.0, 2.0)), 3.0, Color32::from_black_alpha(150));
                            painter.galley(at, galley, col);
                        }
                    }
                    // Repère d'ancrage.
                    for &(ax, ay) in &anchors {
                        let c = img_rect.min + vec2(ax, ay) * sc;
                        let (dark, light) = (Stroke::new(3.0, Color32::from_black_alpha(200)), Stroke::new(1.5, Color32::from_rgb(255, 80, 60)));
                        let style = self.settings.origin;
                        if style == OriginStyle::Dot {
                            painter.circle_filled(c, 4.5, Color32::from_black_alpha(200));
                            painter.circle_filled(c, 3.0, light.color);
                        }
                        for st in [dark, light] {
                            if style == OriginStyle::Target {
                                painter.circle_stroke(c, 6.0, st);
                            }
                            if matches!(style, OriginStyle::Cross | OriginStyle::Target) {
                                painter.line_segment([c - vec2(14.0, 0.0), c + vec2(14.0, 0.0)], st);
                                painter.line_segment([c - vec2(0.0, 14.0), c + vec2(0.0, 14.0)], st);
                            }
                        }
                    }
                }
                // Pixel sous une position de l'écran : coordonnées, couleur, indice de palette.
                let pixel_at = |hp: egui::Pos2| -> Option<crate::view::PixelInfo> {
                    let tx = (((hp.x - img_rect.min.x) / sc).floor().max(0.0) as usize).min((w as usize).saturating_sub(1));
                    let ty = (((hp.y - img_rect.min.y) / sc).floor().max(0.0) as usize).min((h as usize).saturating_sub(1));
                    let (sx, sy) = self.transform.source_of(tx, ty, cw0 as usize, ch0 as usize);
                    self.shown.as_ref().and_then(|s| match &grid {
                        Some(g) => g.cell_at(sx as i32, sy as i32).and_then(|(d, lx, ly)| {
                            let pic = &s.loaded.pic;
                            let b = pic.dir_bounds(d);
                            pixel_info(pic, d, s.frame.min(pic.dir_len(d).saturating_sub(1)), lx - (b.0 - g.x0), ly - (b.1 - g.y0))
                        }),
                        None => pixel_info(&s.loaded.pic, s.dir, s.frame, sx as i32, sy as i32),
                    })
                };
                // Un clic sur une direction (toutes les directions affichées) la choisit.
                if resp.clicked()
                    && let (Some(g), Some(hp)) = (&grid, resp.interact_pointer_pos())
                    && img_rect.contains(hp)
                {
                    let tx = (((hp.x - img_rect.min.x) / sc).floor().max(0.0) as usize).min((w as usize).saturating_sub(1));
                    let ty = (((hp.y - img_rect.min.y) / sc).floor().max(0.0) as usize).min((h as usize).saturating_sub(1));
                    let (sx, sy) = self.transform.source_of(tx, ty, cw0 as usize, ch0 as usize);
                    select_dir = g.cell_at(sx as i32, sy as i32).map(|(d, _, _)| d);
                }
                // Alt + clic gauche : couleur 1 ; Alt + clic droit : couleur 2.
                if ctx.input(|i| i.modifiers.alt)
                    && (resp.clicked() || resp.secondary_clicked())
                    && let Some(hp) = resp.interact_pointer_pos()
                    && img_rect.contains(hp)
                    && let Some(info) = pixel_at(hp)
                {
                    pick_color = Some((resp.secondary_clicked(), info.rgba));
                }
                // Pixel sous le curseur : position, couleur, indice de palette.
                if self.settings.show_pixel_info
                    && let Some(hp) = resp.hover_pos()
                    && img_rect.contains(hp)
                    && let Some(info) = pixel_at(hp)
                {
                    let mut text = format!("{}, {}   {}", info.x, info.y, hex_color(info.rgba));
                    if let Some(i) = info.index {
                        text += &format!("   idx {i}");
                    }
                    let galley = painter.layout_no_wrap(text, FontId::monospace(13.0), Color32::WHITE);
                    let sz = galley.size();
                    let bubble = Rect::from_min_size(rect.left_bottom() + vec2(10.0, -18.0 - sz.y), sz + vec2(sz.y + 16.0, 8.0));
                    painter.rect_filled(bubble, 4.0, Color32::from_black_alpha(190));
                    let swatch = Rect::from_min_size(bubble.min + vec2(5.0, 4.0), vec2(sz.y, sz.y));
                    painter.rect_filled(swatch, 2.0, Color32::from_rgb(info.rgba[0], info.rgba[1], info.rgba[2]));
                    painter.galley(bubble.min + vec2(sz.y + 12.0, 4.0), galley, Color32::WHITE);
                }
            }
            if let Some((second, c)) = pick_color {
                if second {
                    self.settings.color2 = c;
                } else {
                    self.settings.color1 = c;
                }
            }
            if let Some(d) = select_dir
                && let Some(s) = &mut self.shown
            {
                s.dir = d;
                s.frame = s.frame.min(s.loaded.pic.dir_len(d).saturating_sub(1));
                s.clock = 0.0;
            }
            self.anim_overlay(&ctx, rect);
        } else if let Some((p, e)) = &self.error {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            painter.text(rect.center() - vec2(0.0, 12.0), Align2::CENTER_CENTER, name, FontId::proportional(16.0), text_strong);
            painter.text(rect.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, e, FontId::proportional(13.0), Color32::from_rgb(230, 110, 100));
        } else if self.wanted.is_none() {
            painter.text(rect.center(), Align2::CENTER_CENTER, tr("s.hint"), FontId::proportional(16.0), text_weak);
        }
        if nav_step != 0 {
            self.step(&ctx, nav_step);
        }
        let mut open_req = false;
        let mut quick: Option<(Format, bool, bool)> = None;
        // Alt enfoncé : pas de menu contextuel (Alt + clic sert à prélever les couleurs).
        if self.shown.is_some() && !ctx.input(|i| i.modifiers.alt) {
            resp.context_menu(|ui| {
                if ui.add(egui::Button::new(tr("m.copy_image")).shortcut_text("Ctrl+C")).clicked() {
                    self.copy_image(&ctx);
                    ui.close();
                }
                if ui.add(egui::Button::new(tr("m.convert")).shortcut_text("Ctrl+E")).clicked() {
                    self.open_convert();
                    ui.close();
                }
                if ui.add(egui::Button::new(tr("m.extract_pal")).shortcut_text("Ctrl+P")).clicked() {
                    self.open_palette_dialog();
                    ui.close();
                }
                // Conversions en un clic, à côté du fichier : inutiles vers le format déjà en place.
                ui.separator();
                let fmt = self.shown.as_ref().map(|s| s.loaded.pic.format);
                for (label, target, with_pal, white) in [("m.quick_frm", Format::Frm, false, false), ("m.quick_frm_pal", Format::Frm, true, false), ("m.quick_frm_pal_death", Format::Frm, true, true), ("m.quick_rix", Format::Rix, false, false)] {
                    if ui.add_enabled(fmt != Some(target), egui::Button::new(tr(label))).clicked() {
                        quick = Some((target, with_pal, white));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.add(egui::Button::new(tr("m.open_default")).shortcut_text("Ctrl+Enter")).clicked() {
                    open_req = true;
                    ui.close();
                }
                ui.separator();
                if ui.add(egui::Button::new(tr("t.zoom_100")).shortcut_text("Enter")).clicked() {
                    self.zoom_actual();
                    ui.close();
                }
            });
        }
        if open_req {
            self.open_in_default_program();
        }
        if let Some((fmt, with_pal, white)) = quick {
            self.quick_convert(fmt, with_pal, white);
        }
        // Erreur de chargement par-dessus une image affichée.
        if self.shown.is_some()
            && let Some((p, e)) = &self.error
        {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let r = Rect::from_min_size(rect.left_top() + vec2(10.0, 10.0), vec2(rect.width() - 20.0, 22.0));
            painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(120, 30, 30, 220));
            painter.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{name} : {e}"), FontId::proportional(13.0), Color32::WHITE);
        }
        if self.is_loading() {
            let c = rect.right_top() + vec2(-24.0, 24.0);
            let t = ctx.input(|i| i.time) as f32;
            for k in 0..8 {
                let a = t * 6.0 + k as f32 * std::f32::consts::TAU / 8.0;
                let alpha = ((k as f32 / 8.0) * 255.0) as u8;
                painter.circle_filled(c + vec2(a.cos(), a.sin()) * 9.0, 2.2, Color32::from_white_alpha(alpha.max(40)));
            }
        }
    }

    fn anim_overlay(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(s) = &self.shown else { return };
        let pic = &s.loaded.pic;
        if !(pic.is_animated() || pic.directions > 1) {
            return;
        }
        let (mut n, dirs) = (pic.dir_len(s.dir), pic.directions);
        let (mut dir, mut frame, mut playing) = (s.dir, s.frame, s.playing);
        egui::Area::new(Id::new("anim_overlay")).order(egui::Order::Foreground).pivot(Align2::CENTER_BOTTOM).fixed_pos(pos2(rect.center().x, rect.bottom() - 12.0)).show(
            ctx,
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if n > 1 {
                            if icon_button(ui, if playing { Icon::Pause } else { Icon::Play }, &tr(if playing { "a.pause" } else { "a.play" }), false, true).clicked() {
                                playing = !playing;
                            }
                            ui.separator();
                            // « Image x / y » : sélection directe de l'image par le curseur, encadré des boutons image précédente / suivante.
                            ui.label(tr("a.frame"));
                            if icon_button(ui, Icon::FramePrev, &tr("a.prev_frame"), false, true).clicked() {
                                frame = (frame + n - 1) % n;
                                playing = false;
                            }
                            let mut f = frame as u32 + 1;
                            ui.spacing_mut().slider_width = 150.0;
                            if ui.add(egui::Slider::new(&mut f, 1..=n as u32).suffix(format!(" / {n}"))).changed() {
                                frame = f as usize - 1;
                                playing = false;
                            }
                            if icon_button(ui, Icon::FrameNext, &tr("a.next_frame"), false, true).clicked() {
                                frame = (frame + 1) % n;
                                playing = false;
                            }
                        }
                        if dirs > 1 {
                            ui.separator();
                            ui.label(tr("a.direction"));
                            if icon_button(ui, Icon::Prev, &tr("a.direction"), false, true).clicked() {
                                dir = (dir + dirs - 1) % dirs;
                            }
                            egui::ComboBox::from_id_salt("dir_combo").selected_text(pic.dir_label(dir)).width(if pic.dir_labels.is_empty() { 50.0 } else { 230.0 }).show_ui(
                                ui,
                                |ui| {
                                    for d in 0..dirs {
                                        ui.selectable_value(&mut dir, d, pic.dir_label(d));
                                    }
                                },
                            );
                            if icon_button(ui, Icon::Next, &tr("a.direction"), false, true).clicked() {
                                dir = (dir + 1) % dirs;
                            }
                            n = pic.dir_len(dir);
                            frame = frame.min(n.saturating_sub(1));
                        }
                    });
                });
            },
        );
        let changed = self.shown.as_ref().is_some_and(|s| s.dir != dir || s.frame != frame || s.playing != playing);
        if changed {
            let fit = self.fit;
            if let Some(s) = &mut self.shown {
                let dir_changed = s.dir != dir;
                s.playing = playing;
                s.dir = dir;
                s.frame = frame;
                s.clock = 0.0;
                if dir_changed {
                    s.tex.clear();
                }
            }
            if !fit {
                // conserver zoom/position
            }
            ctx.request_repaint_after(Duration::from_millis(1));
        }
    }
}
