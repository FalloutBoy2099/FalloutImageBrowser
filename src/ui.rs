//! Interface : menus, barre d'outils, arborescence, propriétés et zone d'affichage.

use crate::app::{App, Smoothing, TexEntry};
use crate::i18n::{self, Lang, tr};
use crate::icons::{Icon, icon_button};
use crate::loader::PalPolicy;
use crate::model::*;
use crate::tree::RowKind;
use eframe::egui::{self, Align2, Color32, ColorImage, FontId, Id, Rect, Sense, Stroke, TextureOptions, Ui, Vec2, pos2, vec2};
use std::time::Duration;

const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 64.0;

/// Taille logique (canevas commun d'une direction pour les animations).
fn canvas_size(pic: &Picture, dir: usize) -> (u32, u32) {
    if pic.frames_per_dir > 1 {
        let (x0, y0, x1, y1) = pic.dir_bounds(dir);
        let (w, h) = ((x1 - x0) as i64, (y1 - y0) as i64);
        if w > 0 && h > 0 && w * h <= 64_000_000 {
            return (w as u32, h as u32);
        }
    }
    let f = &pic.frames[pic.frame_index(dir, 0)];
    (f.width, f.height)
}

fn render_rgba(pic: &Picture, dir: usize, f: usize) -> (usize, usize, Vec<u8>) {
    let frame = &pic.frames[pic.frame_index(dir, f)];
    let (cw, ch) = canvas_size(pic, dir);
    let rgba = frame_to_rgba(frame, pic.palette.as_ref());
    if pic.frames_per_dir <= 1 || (cw == frame.width && ch == frame.height) {
        return (frame.width as usize, frame.height as usize, rgba);
    }
    let (x0, y0, _, _) = pic.dir_bounds(dir);
    let (cw, ch) = (cw as usize, ch as usize);
    let mut canvas = vec![0u8; cw * ch * 4];
    let (ox, oy) = ((frame.pos.0 - x0) as usize, (frame.pos.1 - y0) as usize);
    for y in 0..frame.height as usize {
        let dst = ((oy + y) * cw + ox) * 4;
        let src = y * frame.width as usize * 4;
        let n = frame.width as usize * 4;
        if dst + n <= canvas.len() {
            canvas[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
        }
    }
    (cw, ch, canvas)
}

/// Réduction par moyenne de blocs k×k (pondérée par l'alpha).
fn box_downscale(rgba: &[u8], w: usize, h: usize, k: usize) -> (Vec<u8>, usize, usize) {
    let (nw, nh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for yy in y * k..((y + 1) * k).min(h) {
                for xx in x * k..((x + 1) * k).min(w) {
                    let p = &rgba[(yy * w + xx) * 4..(yy * w + xx) * 4 + 4];
                    let al = p[3] as u64;
                    r += p[0] as u64 * al;
                    g += p[1] as u64 * al;
                    b += p[2] as u64 * al;
                    a += al;
                    n += 1;
                }
            }
            let o = (y * nw + x) * 4;
            if a > 0 {
                out[o] = (r / a) as u8;
                out[o + 1] = (g / a) as u8;
                out[o + 2] = (b / a) as u8;
            }
            out[o + 3] = (a / n.max(1)) as u8;
        }
    }
    (out, nw, nh)
}

impl App {
    // ------------------------------------------------------------------ entrées

    pub fn handle_input(&mut self, ctx: &egui::Context) {
        // Échap ferme les fenêtres d'information (aide de la ligne de commande, À propos).
        if !self.conv.open && !self.pal_dlg.open && self.expand_confirm.is_none() && self.folder_dlg.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_cli_help {
                self.show_cli_help = false;
                return;
            }
            if self.show_about {
                self.show_about = false;
                return;
            }
        }
        // La fenêtre de conversion est modale : rien d'autre ne réagit.
        if self.conv.open || self.pal_dlg.open || self.expand_confirm.is_some() || self.folder_dlg.is_some() {
            return;
        }
        // Dépôt de fichiers / dossiers : un fichier ouvre son dossier, qui devient le dossier courant.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.first() {
            self.open_path(ctx, p);
        }
        // Boutons latéraux de la souris : précédent / suivant (par défaut) ou zoom.
        let (back, fwd) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if self.settings.xbtn_zoom {
            if back {
                self.zoom_by(0.8);
            }
            if fwd {
                self.zoom_by(1.25);
            }
        } else {
            if back {
                self.step(ctx, -1);
            }
            if fwd {
                self.step(ctx, 1);
            }
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::Key;
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        let ctrl = ctx.input(|i| i.modifiers.command);
        let alt = ctx.input(|i| i.modifiers.alt);
        if ctrl && !alt {
            // Raccourcis des menus (affichés dans les menus).
            if pressed(Key::N) {
                self.pick_folder(ctx);
            } else if pressed(Key::O) {
                self.pick_file(ctx);
            } else if pressed(Key::D) {
                self.pick_dat(ctx);
            } else if pressed(Key::L) {
                self.open_folder_dialog();
            } else if pressed(Key::E) {
                self.open_convert();
            } else if pressed(Key::P) {
                self.open_palette_dialog();
            } else if pressed(Key::Enter) {
                self.open_in_default_program();
            } else if pressed(Key::Q) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if pressed(Key::Num1) {
                self.set_policy(ctx, PalPolicy::Auto);
            } else if pressed(Key::Num2) {
                self.set_policy(ctx, PalPolicy::Fallout);
            } else if pressed(Key::Num3) {
                self.load_custom_palette(ctx);
            }
            return;
        }
        if pressed(Key::F1) {
            self.show_about = true;
        }
        if pressed(Key::Space) || pressed(Key::ArrowRight) || pressed(Key::ArrowUp) || pressed(Key::PageDown) {
            self.step(ctx, 1);
        }
        if pressed(Key::Backspace) || pressed(Key::ArrowLeft) || pressed(Key::ArrowDown) || pressed(Key::PageUp) {
            self.step(ctx, -1);
        }
        if pressed(Key::Home) {
            self.goto(ctx, 0);
        }
        if pressed(Key::End) && !self.files.is_empty() {
            self.goto(ctx, self.files.len() - 1);
        }
        if pressed(Key::Plus) || pressed(Key::Equals) {
            self.zoom_by(1.25);
        }
        if pressed(Key::Minus) {
            self.zoom_by(0.8);
        }
        if pressed(Key::Enter) {
            self.zoom_actual();
        }
        if pressed(Key::F) {
            self.fit = true;
            self.pan = Vec2::ZERO;
        }
        if pressed(Key::P) {
            if let Some(s) = &mut self.shown {
                s.playing = !s.playing;
            }
        }
        if pressed(Key::F5) {
            self.reload_dir(ctx);
        }
        if pressed(Key::I) {
            self.settings.show_props = !self.settings.show_props;
        }
        if pressed(Key::T) {
            self.settings.show_tree = !self.settings.show_tree;
        }
    }

    pub fn open_in_default_program(&mut self) {
        if let Some(p) = self.shown.as_ref().map(|s| s.loaded.path.clone()) {
            if let Err(e) = crate::opener::open_default(&p) {
                self.toast(e);
            }
        }
    }

    pub fn zoom_by(&mut self, f: f32) {
        self.fit = false;
        self.zoom = (self.zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= f;
    }

    pub fn zoom_actual(&mut self) {
        self.fit = false;
        let r = 1.0 / self.zoom.max(1e-6);
        self.zoom = 1.0;
        self.pan *= r;
    }

    pub fn pick_folder(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_folder"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.open_dir(ctx, &p, true);
        }
    }

    pub fn pick_file(&mut self, ctx: &egui::Context) {
        let exts: Vec<&str> = vec![
            "art", "bmp", "frm", "fr0", "fr1", "fr2", "fr3", "fr4", "fr5", "gif", "jpg", "jpeg", "pcx", "rix", "tif", "tiff", "tga", "webp", "png", "psd", "psb", "pdn", "xcf", "kra", "ora", "dds", "qoi", "pbm", "pgm", "ppm", "pnm", "pam", "ase", "aseprite", "pal", "act", "gpl", "hex", "kpl", "zar", "til", "spr",
        ];
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_file")).add_filter(tr("m.images"), &exts);
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p);
        }
    }

    pub fn pick_dat(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_dat")).add_filter("DAT", &["dat", "DAT"]);
        if let Some(dir) = &self.settings.last_dir {
            if let Some(p) = crate::dat::real_parent(dir) {
                d = d.set_directory(p);
            }
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p);
        }
    }

    // ------------------------------------------------------------------ menu

    pub fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // Élément de menu avec son raccourci affiché à droite.
        fn item(ui: &mut Ui, label: String, shortcut: &str, enabled: bool) -> bool {
            let r = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut));
            if r.clicked() {
                ui.close();
            }
            r.clicked()
        }
        let has_img = self.shown.is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("m.file"), |ui| {
                if item(ui, tr("m.open_folder"), "Ctrl+N", true) {
                    self.pick_folder(&ctx);
                }
                if item(ui, tr("m.open_file"), "Ctrl+O", true) {
                    self.pick_file(&ctx);
                }
                if item(ui, tr("m.open_dat"), "Ctrl+D", true) {
                    self.pick_dat(&ctx);
                }
                if item(ui, tr("m.reload"), "F5", true) {
                    self.reload_dir(&ctx);
                }
                ui.separator();
                if item(ui, tr("m.convert"), "Ctrl+E", has_img) {
                    self.open_convert();
                }
                if item(ui, tr("m.extract_pal"), "Ctrl+P", has_img) {
                    self.open_palette_dialog();
                }
                ui.separator();
                if item(ui, tr("m.open_default"), "Ctrl+Enter", has_img) {
                    self.open_in_default_program();
                }
                ui.separator();
                if item(ui, tr("m.exit"), "Ctrl+Q", true) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(tr("m.view"), |ui| {
                ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
                ui.checkbox(&mut self.settings.show_props, tr("properties"));
                ui.separator();
                if item(ui, tr("m.fit"), "F", has_img) {
                    self.fit = true;
                    self.pan = Vec2::ZERO;
                }
                ui.separator();
                ui.checkbox(&mut self.settings.autoplay, tr("m.autoplay"));
                ui.separator();
                ui.label(tr("m.wheel"));
                ui.radio_value(&mut self.settings.wheel_nav, false, tr("m.wheel_zoom"));
                ui.radio_value(&mut self.settings.wheel_nav, true, tr("m.wheel_nav"));
                ui.separator();
                ui.label(tr("m.xbuttons"));
                ui.radio_value(&mut self.settings.xbtn_zoom, false, tr("m.xb_nav"));
                ui.radio_value(&mut self.settings.xbtn_zoom, true, tr("m.xb_zoom"));
                ui.separator();
                ui.checkbox(&mut self.win.remember_size, tr("m.remember_size"));
                ui.checkbox(&mut self.win.remember_pos, tr("m.remember_pos"));
                ui.separator();
                ui.label(tr("m.smoothing"));
                for (s, k) in [(Smoothing::Auto, "m.smooth_auto"), (Smoothing::On, "m.smooth_on"), (Smoothing::Off, "m.smooth_off")] {
                    if ui.radio_value(&mut self.settings.smoothing, s, tr(k)).clicked() {
                        if let Some(sh) = &mut self.shown {
                            sh.tex.clear();
                        }
                    }
                }
            });
            ui.menu_button(tr("m.palette"), |ui| {
                let mode = self.settings.pal_mode;
                if ui.radio(matches!(self.policy, PalPolicy::Auto), format!("{}   (Ctrl+1)", tr("m.pal_auto"))).clicked() {
                    self.set_policy(&ctx, PalPolicy::Auto);
                    ui.close();
                }
                if ui.radio(matches!(self.policy, PalPolicy::Fallout), format!("{}   (Ctrl+2)", tr("m.pal_fallout"))).clicked() {
                    self.set_policy(&ctx, PalPolicy::Fallout);
                    ui.close();
                }
                let custom_label = match &self.policy {
                    PalPolicy::Custom(_, n) => format!("{} ({n})   (Ctrl+3)", tr("m.pal_load")),
                    _ => format!("{}   (Ctrl+3)", tr("m.pal_load")),
                };
                if ui.radio(mode == 2 && matches!(self.policy, PalPolicy::Custom(..)), custom_label).clicked() {
                    ui.close();
                    self.load_custom_palette(&ctx);
                }
            });
            ui.menu_button(tr("m.language"), |ui| {
                for l in Lang::ALL {
                    if ui.radio(i18n::lang() == l, l.native_name()).clicked() {
                        self.set_language(&ctx, l);
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr("m.help"), |ui| {
                if item(ui, tr("m.cli_help"), "", true) {
                    self.show_cli_help = true;
                }
                if item(ui, tr("m.about"), "F1", true) {
                    self.show_about = true;
                }
            });
        });
    }

    // ------------------------------------------------------------------ barre d'outils

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
                    if icon_button(ui, Icon::ZoomIn, &tr("t.zoom_in"), false, has_img).clicked() {
                        self.zoom_by(1.25);
                    }
                    if icon_button(ui, Icon::ZoomOut, &tr("t.zoom_out"), false, has_img).clicked() {
                        self.zoom_by(0.8);
                    }
                    if icon_button(ui, Icon::Actual, &tr("t.zoom_100"), !self.fit && (self.zoom - 1.0).abs() < 1e-3, has_img).clicked() {
                        self.zoom_actual();
                    }
                    if icon_button(ui, Icon::Fit, &tr("m.fit"), self.fit, has_img).clicked() {
                        self.fit = true;
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

    // ------------------------------------------------------------------ arborescence

    pub fn tree_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let total = self.tree.rows.len();
        let cur = self.wanted.clone();
        let sel_dir = self.tree.selected_dir.clone();
        let mut action: Option<(usize, bool)> = None; // (ligne, clic sur la flèche)
        let mut ctx_act: Option<(std::path::PathBuf, u8)> = None;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(r) = self.tree_scroll_to.take() {
            let target = (r as f32 * row_h - ui.available_height() * 0.5).max(0.0);
            area = area.vertical_scroll_offset(target);
        }
        area.show_rows(ui, row_h, total, |ui, range| {
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
                let text_x;
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
                        text_x = fx + 18.0;
                    }
                    RowKind::File => {
                        let fx = x0 + 15.0;
                        let col = if selected { text_col } else { vis.weak_text_color() };
                        ui.painter().rect_stroke(Rect::from_min_size(pos2(fx + 1.0, cy - 6.0), vec2(10.0, 12.0)), 1.5, Stroke::new(1.2, col), egui::StrokeKind::Inside);
                        ui.painter().line_segment([pos2(fx + 3.0, cy + 2.0), pos2(fx + 6.0, cy - 1.0)], Stroke::new(1.0, col));
                        ui.painter().line_segment([pos2(fx + 6.0, cy - 1.0), pos2(fx + 9.0, cy + 2.0)], Stroke::new(1.0, col));
                        text_x = fx + 18.0;
                    }
                }
                let clip = ui.painter().with_clip_rect(rect);
                let font = FontId::proportional(13.5);
                clip.text(pos2(text_x, cy), Align2::LEFT_CENTER, &row.name, font, if is_sel_dir && !selected { vis.strong_text_color() } else { text_col });
                if resp.clicked() {
                    let on_arrow = matches!(row.kind, RowKind::Dir { .. })
                        && resp.interact_pointer_pos().is_some_and(|p| p.x < x0 + 14.0);
                    action = Some((i, on_arrow));
                }
                let is_dir = matches!(row.kind, RowKind::Dir { .. });
                resp.context_menu(|ui| {
                    if ui.add_enabled(is_dir, egui::Button::new(tr("t.expand_all"))).clicked() {
                        ctx_act = Some((row.path.clone(), 0));
                        ui.close();
                    }
                    if ui.button(tr("t.collapse_others")).clicked() {
                        ctx_act = Some((row.path.clone(), 1));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(tr("t.open_explorer")).clicked() {
                        ctx_act = Some((row.path.clone(), 2));
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

    // ------------------------------------------------------------------ propriétés

    pub fn props_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.heading(tr("properties"));
        ui.separator();
        let Some(s) = &self.shown else {
            ui.label(egui::RichText::new(tr("s.no_image")).weak());
            return;
        };
        let pic = &s.loaded.pic;
        // Boutons du bas : copier toutes les propriétés en texte ou en JSON.
        let mut copy: Option<bool> = None;
        egui::Panel::bottom("props_copy").show(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                if icon_button(ui, Icon::CopyText, &tr("t.copy_text"), false, true).clicked() {
                    copy = Some(false);
                }
                if icon_button(ui, Icon::CopyJson, &tr("t.copy_json"), false, true).clicked() {
                    copy = Some(true);
                }
            });
            ui.add_space(3.0);
        });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut sid = 0;
            for sec in &pic.sections {
                sid += 1;
                egui::CollapsingHeader::new(egui::RichText::new(tr(&sec.title)).strong())
                    .id_salt(("sec", &sec.title))
                    .default_open(sec.title != "sec.exif")
                    .show(ui, |ui| {
                        egui::Grid::new(("grid", sid)).num_columns(2).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                            for p in &sec.items {
                                ui.label(egui::RichText::new(tr(&p.key)).weak());
                                let v = if p.value.starts_with("p.") || p.value.starts_with("c.") { tr(&p.value) } else { p.value.clone() };
                                ui.add(egui::Label::new(v).wrap().selectable(true));
                                ui.end_row();
                            }
                        });
                    });
            }
            // Image courante (animations / directions).
            if pic.frames.len() > 1 {
                egui::CollapsingHeader::new(egui::RichText::new(tr("sec.current")).strong()).default_open(true).show(ui, |ui| {
                    let f = &pic.frames[pic.frame_index(s.dir, s.frame)];
                    egui::Grid::new("grid_cur").num_columns(2).striped(true).show(ui, |ui| {
                        if pic.directions > 1 {
                            ui.label(egui::RichText::new(tr("p.direction")).weak());
                            ui.label(format!("{} / {}", s.dir + 1, pic.directions));
                            ui.end_row();
                        }
                        ui.label(egui::RichText::new(tr("a.frame")).weak());
                        ui.label(format!("{} / {}", s.frame + 1, pic.dir_len(s.dir)));
                        ui.end_row();
                        ui.label(egui::RichText::new(tr("p.dimensions")).weak());
                        ui.label(format!("{} × {}", f.width, f.height));
                        ui.end_row();
                        ui.label(egui::RichText::new(tr("p.offset")).weak());
                        ui.label(format!("{}, {}", f.pos.0, f.pos.1));
                        ui.end_row();
                        if let Some(d) = f.delay_ms {
                            ui.label(egui::RichText::new(tr("p.delay")).weak());
                            ui.label(format!("{d} ms"));
                            ui.end_row();
                        }
                    });
                });
            }
            if let Some(pal) = pic.palette.as_ref().or(pic.colors.as_ref()) {
                egui::CollapsingHeader::new(egui::RichText::new(tr("sec.palette")).strong()).default_open(true).show(ui, |ui| {
                    palette_grid(ui, pal);
                });
            }
        });
        if let Some(json) = copy {
            let sections = collect_props(pic, s.dir, s.frame);
            let out = if json { props_json(&sections) } else { props_text(&sections) };
            ui.ctx().copy_text(out);
            self.toast(tr("t.copied"));
        }
    }

    // ------------------------------------------------------------------ canevas

    fn checker_id(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.checker.is_none() {
            let (a, b) = (Color32::from_gray(0x70), Color32::from_gray(0x88));
            let mut px = Vec::with_capacity(256);
            for y in 0..16 {
                for x in 0..16 {
                    px.push(if (x / 8 + y / 8) % 2 == 0 { a } else { b });
                }
            }
            let img = ColorImage { size: [16, 16], source_size: Vec2::new(16.0, 16.0), pixels: px };
            let opts = TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Nearest, wrap_mode: egui::TextureWrapMode::Repeat, mipmap_mode: None };
            self.checker = Some(ctx.load_texture("checker", img, opts));
        }
        self.checker.as_ref().unwrap().id()
    }

    fn ensure_tex(&mut self, ctx: &egui::Context, level: u8) -> Option<(egui::TextureId, u32, u32)> {
        let smoothing = self.settings.smoothing;
        let s = self.shown.as_mut()?;
        let pic = &s.loaded.pic;
        let (w, h) = canvas_size(pic, s.dir);
        let key = (s.dir, s.frame, level);
        if !s.tex.contains_key(&key) {
            let cap = if pic.is_animated() { 96 } else { 8 };
            if s.tex.len() >= cap {
                s.tex.clear();
            }
            let (cw, ch, rgba) = render_rgba(pic, s.dir, s.frame);
            let k = 1usize << level;
            let (rgba, tw, th) = if k > 1 { box_downscale(&rgba, cw, ch, k) } else { (rgba, cw, ch) };
            let img = ColorImage::from_rgba_unmultiplied([tw.max(1), th.max(1)], &rgba);
            let pixelated = matches!(pic.frames[0].pixels, Pixels::Indexed(_)) || w.max(h) <= 512;
            let mag = match smoothing {
                Smoothing::Auto if pixelated => egui::TextureFilter::Nearest,
                Smoothing::Off => egui::TextureFilter::Nearest,
                _ => egui::TextureFilter::Linear,
            };
            let min = if smoothing == Smoothing::Off { egui::TextureFilter::Nearest } else { egui::TextureFilter::Linear };
            let handle = ctx.load_texture(
                format!("img-{}-{}-{}", s.dir, s.frame, level),
                img,
                TextureOptions { magnification: mag, minification: min, wrap_mode: egui::TextureWrapMode::ClampToEdge, mipmap_mode: None },
            );
            s.tex.insert(key, TexEntry { handle });
        }
        Some((s.tex[&key].handle.id(), w, h))
    }

    pub fn canvas(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        let mut nav_step = 0i32;

        if let Some(s) = &self.shown {
            let (iw, ih) = canvas_size(&s.loaded.pic, s.dir);
            let (iw, ih) = (iw as f32, ih as f32);
            // Zoom « ajusté ».
            if self.fit {
                let z = (rect.width() * ppp / iw).min(rect.height() * ppp / ih).min(1.0);
                self.zoom = z.clamp(MIN_ZOOM, MAX_ZOOM);
                self.pan = Vec2::ZERO;
            }
            // Interactions.
            if resp.dragged() {
                if self.fit {
                    self.fit = false;
                }
                self.pan += resp.drag_delta();
            }
            // Clic de molette (molette en mode zoom) : retour à 100 %.
            if !self.settings.wheel_nav && resp.hovered() && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Middle)) {
                self.zoom_actual();
            }
            if resp.double_clicked() {
                if self.fit && self.zoom < 1.0 {
                    self.zoom_actual();
                } else if self.fit {
                    // déjà à 100 %
                } else {
                    self.fit = true;
                }
            }
            if resp.hovered() {
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
                if factor != 1.0 {
                    if let Some(cur) = resp.hover_pos() {
                        let old = self.zoom;
                        let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                        let ratio = new / old;
                        let rel = cur - rect.center();
                        self.pan = rel - (rel - self.pan) * ratio;
                        self.zoom = new;
                        self.fit = false;
                    }
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
            let tex = self.ensure_tex(&ctx, level);
            if let Some((tid, w, h)) = tex {
                let size_pts = vec2(w as f32, h as f32) * self.zoom / ppp;
                let snap = |v: f32| (v * ppp).round() / ppp;
                let mut min = rect.center() + self.pan - size_pts * 0.5;
                min = pos2(snap(min.x), snap(min.y));
                let img_rect = Rect::from_min_size(min, size_pts);
                let vis = img_rect.intersect(rect);
                if vis.is_positive() {
                    let ck = self.checker_id(&ctx);
                    let uv = Rect::from_min_max(
                        pos2((vis.min.x - img_rect.min.x) * ppp / 16.0, (vis.min.y - img_rect.min.y) * ppp / 16.0),
                        pos2((vis.max.x - img_rect.min.x) * ppp / 16.0, (vis.max.y - img_rect.min.y) * ppp / 16.0),
                    );
                    painter.image(ck, vis, uv, Color32::WHITE);
                    painter.image(tid, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
            self.anim_overlay(&ctx, rect);
        } else if let Some((p, e)) = &self.error {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            painter.text(rect.center() - vec2(0.0, 12.0), Align2::CENTER_CENTER, name, FontId::proportional(16.0), ui.visuals().strong_text_color());
            painter.text(rect.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, e, FontId::proportional(13.0), Color32::from_rgb(230, 110, 100));
        } else if self.wanted.is_none() {
            painter.text(rect.center(), Align2::CENTER_CENTER, tr("s.hint"), FontId::proportional(16.0), ui.visuals().weak_text_color());
        }
        if nav_step != 0 {
            self.step(&ctx, nav_step);
        }
        let mut open_req = false;
        if self.shown.is_some() {
            resp.context_menu(|ui| {
                if ui.add(egui::Button::new(tr("m.convert")).shortcut_text("Ctrl+E")).clicked() {
                    self.open_convert();
                    ui.close();
                }
                if ui.add(egui::Button::new(tr("m.extract_pal")).shortcut_text("Ctrl+P")).clicked() {
                    self.open_palette_dialog();
                    ui.close();
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
        // Erreur de chargement par-dessus une image affichée.
        if self.shown.is_some() {
            if let Some((p, e)) = &self.error {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let r = Rect::from_min_size(rect.left_top() + vec2(10.0, 10.0), vec2(rect.width() - 20.0, 22.0));
                painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(120, 30, 30, 220));
                painter.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{name} : {e}"), FontId::proportional(13.0), Color32::WHITE);
            }
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
        egui::Area::new(Id::new("anim_overlay"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(rect.center().x, rect.bottom() - 12.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if n > 1 {
                            if icon_button(ui, Icon::Prev, &tr("a.frame"), false, true).clicked() {
                                frame = (frame + n - 1) % n;
                                playing = false;
                            }
                            if icon_button(ui, if playing { Icon::Pause } else { Icon::Play }, &tr(if playing { "a.pause" } else { "a.play" }), false, true).clicked() {
                                playing = !playing;
                            }
                            if icon_button(ui, Icon::Next, &tr("a.frame"), false, true).clicked() {
                                frame = (frame + 1) % n;
                                playing = false;
                            }
                            // « Image x / y » : sélection directe de l'image par le curseur.
                            ui.label(tr("a.frame"));
                            let mut f = frame as u32 + 1;
                            ui.spacing_mut().slider_width = 150.0;
                            if ui.add(egui::Slider::new(&mut f, 1..=n as u32).suffix(format!(" / {n}"))).changed() {
                                frame = f as usize - 1;
                                playing = false;
                            }
                        }
                        if dirs > 1 {
                            ui.separator();
                            ui.label(tr("a.direction"));
                            egui::ComboBox::from_id_salt("dir_combo").selected_text(pic.dir_label(dir)).width(if pic.dir_labels.is_empty() { 50.0 } else { 230.0 }).show_ui(ui, |ui| {
                                for d in 0..dirs {
                                    ui.selectable_value(&mut dir, d, pic.dir_label(d));
                                }
                            });
                            n = pic.dir_len(dir);
                            frame = frame.min(n.saturating_sub(1));
                        }
                    });
                });
            });
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

    // ------------------------------------------------------------------ fenêtres annexes

    pub fn cli_help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_cli_help;
        egui::Window::new(tr("m.cli_help")).open(&mut open).collapsible(false).resizable(true).default_size([760.0, 560.0]).show(ctx, |ui| {
            ui.add(egui::Label::new(tr("cli.intro")).wrap());
            ui.horizontal(|ui| {
                if ui.button(tr("c.copy")).clicked() {
                    ctx.copy_text(crate::cli::usage());
                }
            });
            ui.separator();
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(crate::cli::usage()).monospace().size(12.5)).extend().selectable(true));
            });
        });
        self.show_cli_help = open;
    }

    /// Ouvre la boîte « Dossier courant » avec le chemin actuel.
    pub fn open_folder_dialog(&mut self) {
        let cur = self.tree.selected_dir.clone().or_else(|| self.settings.last_dir.clone());
        self.folder_dlg = Some((cur.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(), true));
    }

    /// Boîte modale affichant le dossier courant, modifiable pour changer rapidement de dossier.
    /// Un dossier inexistant n'est pas pris en compte.
    pub fn folder_window(&mut self, ctx: &egui::Context) {
        let Some((mut text, fresh)) = self.folder_dlg.take() else { return };
        let path = std::path::PathBuf::from(text.trim().trim_matches('"'));
        let valid = !text.trim().is_empty() && (matches!(crate::dat::stat(&path), Some(crate::dat::Stat::Dir)) || path.is_dir());
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("folder_dlg")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(tr("t.folder"));
            ui.add_space(4.0);
            let mut out = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY).show(ui);
            if fresh {
                out.response.request_focus();
                out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
                out.state.store(ui.ctx(), out.response.id);
            }
            if out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid {
                go = true;
            }
            if !valid && !text.trim().is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.folder_missing"));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                go |= ui.add_enabled(valid, egui::Button::new(tr("c.ok"))).clicked();
                cancel = ui.button(tr("c.cancel")).clicked();
            });
        });
        if go {
            self.open_path(ctx, &path);
        } else if !cancel && !modal.should_close() {
            self.folder_dlg = Some((text, false));
        }
    }

    /// Confirmation avant de déplier plus de 100 dossiers.
    pub fn expand_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.expand_confirm.clone() else { return };
        let (mut yes, mut no) = (false, false);
        let modal = egui::Modal::new(Id::new("expand_confirm")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(tr("t.expand_all"));
            ui.add(egui::Label::new(tr("t.expand_confirm")).wrap());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                yes = ui.button(tr("t.expand_all")).clicked();
                no = ui.button(tr("c.cancel")).clicked();
            });
        });
        if yes {
            self.tree.expand_all(&path);
        }
        if yes || no || modal.should_close() {
            self.expand_confirm = None;
        }
    }

    pub fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new(tr("m.about")).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.set_min_width(480.0);
            ui.horizontal(|ui| {
                ui.heading("Fallout Image Browser");
                ui.label(egui::RichText::new("by Kain").italics());
            });
            ui.label(egui::RichText::new(format!("v{}  ·  Rust + egui + wgpu", env!("CARGO_PKG_VERSION"))).weak());
            ui.label(egui::RichText::new(tr("about.based")).weak());
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new(tr("about.formats")).strong());
            ui.add(egui::Label::new(Format::ALL.iter().map(|f| f.name()).collect::<Vec<_>>().join("  ·  ")).extend());
            ui.label(egui::RichText::new(tr("about.readonly")).weak().small());
            ui.add(egui::Label::new(Format::READ_ONLY.iter().filter(|f| **f != Format::Palette).map(|f| f.name()).collect::<Vec<_>>().join("  ·  ")).extend());
            ui.label(egui::RichText::new(tr("about.palettes")).weak().small());
            ui.label("PAL  ·  ACT  ·  GPL  ·  KPL  ·  HEX");
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new(tr("about.keys_title")).strong());
            ui.label(tr("about.keys"));
        });
        self.show_about = open;
    }

    pub fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some((msg, t)) = &self.toast else { return };
        if t.elapsed() > Duration::from_secs(6) {
            self.toast = None;
            return;
        }
        let msg = msg.clone();
        egui::Area::new(Id::new("toast")).order(egui::Order::Tooltip).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -48.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(msg);
            });
        });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

type PropSections = Vec<(String, Vec<(String, String)>)>;

/// Toutes les propriétés affichées dans le panneau (sections, image courante, palette), textes traduits.
fn collect_props(pic: &Picture, dir: usize, frame: usize) -> PropSections {
    let mut out: PropSections = Vec::new();
    for sec in &pic.sections {
        let rows = sec
            .items
            .iter()
            .map(|p| (tr(&p.key), if p.value.starts_with("p.") || p.value.starts_with("c.") { tr(&p.value) } else { p.value.clone() }))
            .collect();
        out.push((tr(&sec.title), rows));
    }
    if pic.frames.len() > 1 {
        let f = &pic.frames[pic.frame_index(dir, frame)];
        let mut rows = Vec::new();
        if pic.directions > 1 {
            rows.push((tr("p.direction"), format!("{} / {}", dir + 1, pic.directions)));
        }
        rows.push((tr("a.frame"), format!("{} / {}", frame + 1, pic.dir_len(dir))));
        rows.push((tr("p.dimensions"), format!("{} × {}", f.width, f.height)));
        rows.push((tr("p.offset"), format!("{}, {}", f.pos.0, f.pos.1)));
        if let Some(d) = f.delay_ms {
            rows.push((tr("p.delay"), format!("{d} ms")));
        }
        out.push((tr("sec.current"), rows));
    }
    if let Some(pal) = pic.palette.as_ref().or(pic.colors.as_ref()) {
        let rows = (0..256).filter(|&i| pal.used[i]).map(|i| (i.to_string(), format!("#{:02X}{:02X}{:02X}", pal.colors[i][0], pal.colors[i][1], pal.colors[i][2]))).collect();
        out.push((tr("sec.palette"), rows));
    }
    out
}

fn props_text(sections: &PropSections) -> String {
    let mut s = String::new();
    for (title, rows) in sections {
        if !s.is_empty() {
            s.push('\n');
        }
        s += &format!("[{title}]\n");
        for (k, v) in rows {
            s += &format!("{k}: {v}\n");
        }
    }
    s
}

fn json_str(v: &str) -> String {
    let mut o = String::with_capacity(v.len() + 2);
    o.push('"');
    for ch in v.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Clé unique dans un objet JSON : un doublon reçoit le suffixe « (2) », « (3) »…
fn unique_key(used: &mut Vec<String>, k: &str) -> String {
    let mut name = k.to_string();
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{k} ({n})");
        n += 1;
    }
    used.push(name.clone());
    name
}

fn props_json(sections: &PropSections) -> String {
    let mut titles: Vec<String> = Vec::new();
    let mut o = String::from("{\n");
    for (si, (title, rows)) in sections.iter().enumerate() {
        o += &format!("  {}: {{\n", json_str(&unique_key(&mut titles, title)));
        let mut keys: Vec<String> = Vec::new();
        for (ri, (k, v)) in rows.iter().enumerate() {
            o += &format!("    {}: {}{}\n", json_str(&unique_key(&mut keys, k)), json_str(v), if ri + 1 < rows.len() { "," } else { "" });
        }
        o += if si + 1 < sections.len() { "  },\n" } else { "  }\n" };
    }
    o.push('}');
    o
}

fn palette_grid(ui: &mut Ui, pal: &Palette) {
    let cell = 12.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(cell * 16.0, cell * 16.0), Sense::hover());
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
    if let Some(p) = resp.hover_pos() {
        let (x, y) = (((p.x - rect.left()) / cell) as usize, ((p.y - rect.top()) / cell) as usize);
        if x < 16 && y < 16 {
            let i = y * 16 + x;
            let c = pal.colors[i];
            resp.on_hover_text(format!("#{i}   RGB {}, {}, {}   #{:02X}{:02X}{:02X}{}", c[0], c[1], c[2], c[0], c[1], c[2], if !pal.used[i] { "  (unused)" } else if c[3] == 0 { "  (transparent)" } else { "" }));
        }
    }
}
