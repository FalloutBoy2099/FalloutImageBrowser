//! Interface : menus, barre d'outils, arborescence, propriétés et zone d'affichage.

use crate::app::{App, MAX_FAVORITES, Smoothing, TexEntry};
use crate::i18n::{self, Lang, tr};
use crate::icons::{Icon, icon_button};
use crate::loader::PalPolicy;
use crate::model::*;
use crate::props::{collect_props, props_json, props_text, props_xml};
use crate::tree::{RowKind, SortKey};
use crate::view::{Background, hex_rgb, hsv_to_rgb, parse_hex, push_recent, rgb_to_hsv};
use crate::view::{anchor_in_canvas, box_downscale, can_show_all_dirs, canvas_size, dir_grid, hex_color, pixel_info, render_all_dirs, render_rgba_cow};
use eframe::egui::{self, Align2, Color32, ColorImage, FontId, Id, Rect, Sense, Stroke, TextureOptions, Ui, Vec2, pos2, vec2};
use rayon::prelude::*;
use std::borrow::Cow;
use std::time::{Duration, Instant};

const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 64.0;

/// Actions des boutons du bas du panneau « Propriétés ».
#[derive(Clone, Copy)]
enum PropAction {
    CopyImage,
    CopyPath,
    CopyTo,
    MoveTo,
    Rename,
    Convert,
    ConvertPalette,
    Text,
    Json,
    Xml,
    Sha,
    ShaStop,
}

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
    // ------------------------------------------------------------------ entrées

    pub fn handle_input(&mut self, ctx: &egui::Context) {
        // Échap ferme les fenêtres d'information (aide de la ligne de commande, À propos).
        if !self.conv.open
            && !self.pal_dlg.open
            && self.expand_confirm.is_none()
            && self.folder_dlg.is_none()
            && self.rename_dlg.is_none()
            && self.anim_dlg.is_none()
            && self.frm_dlg.is_none()
            && !self.bg_picker_open
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
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
        if self.conv.open
            || self.pal_dlg.open
            || self.expand_confirm.is_some()
            || self.folder_dlg.is_some()
            || self.rename_dlg.is_some()
            || self.anim_dlg.is_some()
            || self.frm_dlg.is_some()
            || self.bg_picker_open
        {
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
        // Saisie au clavier au-dessus de l'arborescence : saute au premier nom qui commence par les caractères tapés.
        let over_tree = self.tree_rect.is_some_and(|r| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| r.contains(p))));
        if over_tree && !ctrl && !alt {
            let typed: String = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect());
            let empty = self.ta_buf.is_empty() || self.ta_time.elapsed() > Duration::from_millis(1200);
            let typed: String = typed.chars().filter(|c| !c.is_control() && (!c.is_whitespace() || !empty)).collect();
            if !typed.is_empty() {
                self.type_ahead(ctx, &typed);
                return;
            }
        }
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
            } else if pressed(Key::C) {
                self.copy_image(ctx);
            } else if pressed(Key::E) {
                self.open_convert();
            } else if pressed(Key::P) {
                self.open_palette_dialog();
            } else if pressed(Key::B) {
                self.open_batch();
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
            self.fit_up = true;
            self.pan = Vec2::ZERO;
        }
        if pressed(Key::P)
            && let Some(s) = &mut self.shown
        {
            s.playing = !s.playing;
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
        // Affichage : rotation, symétries, grille, fond, pelure d'oignon, ancrage, diaporama, plein écran.
        if pressed(Key::R) {
            if ctx.input(|i| i.modifiers.shift) {
                self.transform.rotate_left();
            } else {
                self.transform.rotate_right();
            }
        }
        if pressed(Key::H) {
            self.transform.flip_h = !self.transform.flip_h;
        }
        if pressed(Key::V) {
            self.transform.flip_v = !self.transform.flip_v;
        }
        if pressed(Key::G) {
            self.settings.show_grid = !self.settings.show_grid;
        }
        if pressed(Key::B) {
            self.settings.background = self.settings.background.next();
        }
        if pressed(Key::O) {
            self.settings.onion = !self.settings.onion;
        }
        if pressed(Key::D) {
            self.settings.show_all_dirs = !self.settings.show_all_dirs;
        }
        if pressed(Key::A) {
            self.settings.show_origin = !self.settings.show_origin;
        }
        if pressed(Key::S) {
            self.toggle_slideshow(ctx);
        }
        if pressed(Key::F11) || (pressed(Key::Escape) && self.fullscreen) {
            self.toggle_fullscreen(ctx);
        }
    }

    /// Recherche au clavier dans l'arborescence : les caractères tapés s'accumulent pendant 1,2 s ; la même lettre répétée
    /// passe à l'élément suivant qui commence par elle.
    fn type_ahead(&mut self, ctx: &egui::Context, typed: &str) {
        if self.ta_time.elapsed() > Duration::from_millis(1200) {
            self.ta_buf.clear();
        }
        self.ta_time = Instant::now();
        self.ta_buf.push_str(&typed.to_lowercase());
        let rows = &self.tree.rows;
        let n = rows.len();
        if n == 0 {
            return;
        }
        let cur = rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::File) && self.wanted.as_deref() == Some(r.path.as_path()))
            .or_else(|| rows.iter().position(|r| self.tree.selected_dir.as_deref() == Some(r.path.as_path())));
        let first = self.ta_buf.chars().next().unwrap_or(' ');
        let repeated = self.ta_buf.chars().count() > 1 && self.ta_buf.chars().all(|c| c == first);
        let (needle, start) = if repeated {
            (first.to_string(), cur.map_or(0, |c| c + 1))
        } else if self.ta_buf.chars().count() == 1 {
            (self.ta_buf.clone(), cur.map_or(0, |c| c + 1))
        } else {
            (self.ta_buf.clone(), cur.unwrap_or(0))
        };
        let found = (0..n).map(|k| (start + k) % n).find(|&i| rows[i].name.to_lowercase().starts_with(&needle));
        let Some(i) = found else { return };
        let row = rows[i].clone();
        match row.kind {
            RowKind::File => self.show_file(ctx, &row.path),
            RowKind::Dir { .. } => self.open_dir(ctx, &row.path, false),
        }
        self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == row.path);
    }

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

    pub fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    pub fn toggle_slideshow(&mut self, ctx: &egui::Context) {
        self.slideshow = !self.slideshow;
        self.slideshow_t = ctx.input(|i| i.time);
    }

    /// Diaporama : image suivante à intervalle régulier, en revenant à la première après la dernière.
    pub fn tick_slideshow(&mut self, ctx: &egui::Context) {
        if !self.slideshow {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.slideshow_t >= self.settings.slideshow_secs.max(1) as f64 {
            self.slideshow_t = now;
            match self.index {
                Some(i) if i + 1 >= self.files.len() => self.goto(ctx, 0),
                _ => self.step(ctx, 1),
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Associe des extensions de fichier à l'application (menu Aide) et affiche le résultat.
    pub fn associate_files(&mut self, types: &[crate::assoc::FileType]) {
        let exts = crate::assoc::list(types);
        match crate::assoc::associate(types) {
            Ok(()) => {
                let key = if cfg!(windows) { "assoc.done_win" } else { "assoc.done" };
                self.toast(i18n::trf(key, &[("exts", exts)]));
            }
            Err(e) => self.toast(i18n::trf("assoc.failed", &[("exts", exts), ("error", e)])),
        }
    }

    /// Copie l'image affichée (image courante d'une animation) dans le presse-papiers du système.
    pub fn copy_image(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.shown else { return };
        let pic = &s.loaded.pic;
        let f = &pic.frames[pic.frame_index(s.dir, s.frame)];
        let rgba = frame_to_rgba(f, pic.palette.as_ref());
        let image = ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &rgba);
        ctx.copy_image(image);
        self.toast(tr("t.image_copied"));
    }

    pub fn open_in_default_program(&mut self) {
        if let Some(p) = self.shown.as_ref().map(|s| s.loaded.path.clone())
            && let Err(e) = crate::opener::open_default(&p)
        {
            self.toast(e);
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

    /// Demande un dossier de sortie puis extrait, dans un thread, le fichier ou dossier d'archive `.dat` choisi.
    pub fn extract_from_archive(&mut self, path: &std::path::Path) {
        if self.extract_job.is_some() {
            return;
        }
        let mut d = rfd::FileDialog::new().set_title(tr("t.extract"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(crate::dat::real_parent(dir).unwrap_or_else(|| dir.clone()));
        }
        let Some(out) = d.pick_folder() else { return };
        let (src, dest) = (path.to_path_buf(), out.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::dat::extract(&src, &dest, false).map_err(|e| e.to_string()));
        });
        self.extract_job = Some((rx, out));
        self.toast(tr("t.extracting"));
    }

    /// Récupère le résultat d'une extraction terminée et l'affiche.
    pub fn poll_extract(&mut self, ctx: &egui::Context) {
        let Some((rx, out)) = &self.extract_job else { return };
        match rx.try_recv() {
            Ok(result) => {
                let dir = out.display().to_string();
                self.extract_job = None;
                match result {
                    Ok(r) => self.toast(i18n::trf("t.extract_done", &[("written", r.written.to_string()), ("skipped", r.skipped.to_string()), ("dir", dir)])),
                    Err(e) => self.toast(i18n::trf("t.extract_failed", &[("error", e)])),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.extract_job = None,
        }
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
            "art", "bmp", "frm", "fr0", "fr1", "fr2", "fr3", "fr4", "fr5", "gif", "jpg", "jpeg", "pcx", "rix", "tif", "tiff", "tga", "webp", "png", "psd", "psb", "pdn", "xcf",
            "kra", "ora", "dds", "qoi", "pbm", "pgm", "ppm", "pnm", "pam", "ase", "aseprite", "pal", "act", "gpl", "hex", "kpl", "zar", "til", "spr",
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
        if let Some(dir) = &self.settings.last_dir
            && let Some(p) = crate::dat::real_parent(dir)
        {
            d = d.set_directory(p);
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
        let is_frm = self.shown.as_ref().is_some_and(|s| s.loaded.pic.format == Format::Frm);
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
                ui.separator();
                if item(ui, tr("t.refresh"), "F5", true) {
                    self.reload_dir(&ctx);
                }
                ui.separator();
                if item(ui, tr("m.copy_image"), "Ctrl+C", has_img) {
                    self.copy_image(&ctx);
                }
                if item(ui, tr("m.convert"), "Ctrl+E", has_img) {
                    self.open_convert();
                }
                if item(ui, tr("m.extract_pal"), "Ctrl+P", has_img) {
                    self.open_palette_dialog();
                }
                if item(ui, tr("m.batch"), "Ctrl+B", true) {
                    self.open_batch();
                }
                if item(ui, tr("m.edit_frm"), "", is_frm) {
                    self.open_frm_editor();
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
            // Affichage : panneaux, ajustement, plein écran, diaporama, lecture et lissage.
            ui.menu_button(tr("m.view"), |ui| {
                ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
                ui.checkbox(&mut self.settings.show_props, tr("properties"));
                ui.separator();
                if item(ui, tr("m.fit"), "F", has_img) {
                    self.fit = true;
                    self.fit_up = true;
                    self.pan = Vec2::ZERO;
                }
                if item(ui, tr("m.fullscreen"), "F11", true) {
                    self.toggle_fullscreen(&ctx);
                }
                let slideshow_label = if self.slideshow { tr("m.slideshow_stop") } else { tr("m.slideshow") };
                if item(ui, slideshow_label, "S", has_img) {
                    self.toggle_slideshow(&ctx);
                }
                ui.menu_button(tr("m.slideshow_delay"), |ui| {
                    for secs in [1u32, 2, 3, 5, 10] {
                        ui.radio_value(&mut self.settings.slideshow_secs, secs, format!("{secs} s"));
                    }
                });
                ui.separator();
                ui.checkbox(&mut self.settings.autoplay, tr("m.autoplay"));
                ui.separator();
                ui.label(tr("m.smoothing"));
                for (s, k) in [(Smoothing::Auto, "m.smooth_auto"), (Smoothing::On, "m.smooth_on"), (Smoothing::Off, "m.smooth_off")] {
                    if ui.radio_value(&mut self.settings.smoothing, s, tr(k)).clicked()
                        && let Some(sh) = &mut self.shown
                    {
                        sh.tex.clear();
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.win.remember_size, tr("m.remember_size"));
                ui.checkbox(&mut self.win.remember_pos, tr("m.remember_pos"));
            });
            // Image : orientation de la vue, repères d'inspection et fond.
            ui.menu_button(tr("m.image"), |ui| {
                if item(ui, tr("m.rotate_right"), "R", has_img) {
                    self.transform.rotate_right();
                }
                if item(ui, tr("m.rotate_left"), "Shift+R", has_img) {
                    self.transform.rotate_left();
                }
                if item(ui, tr("m.flip_h"), "H", has_img) {
                    self.transform.flip_h = !self.transform.flip_h;
                }
                if item(ui, tr("m.flip_v"), "V", has_img) {
                    self.transform.flip_v = !self.transform.flip_v;
                }
                if item(ui, tr("m.reset_view"), "", has_img && !self.transform.is_identity()) {
                    self.transform = Default::default();
                }
                ui.separator();
                ui.checkbox(&mut self.settings.show_grid, format!("{}  [G]", tr("m.grid")));
                ui.checkbox(&mut self.settings.show_pixel_info, tr("m.pixel_info"));
                ui.checkbox(&mut self.settings.show_origin, format!("{}  [A]", tr("m.origin")));
                ui.checkbox(&mut self.settings.onion, format!("{}  [O]", tr("m.onion")));
                let can_all = self.shown.as_ref().is_some_and(|s| can_show_all_dirs(&s.loaded.pic));
                ui.add_enabled(can_all, egui::Checkbox::new(&mut self.settings.show_all_dirs, format!("{}  [D]", tr("m.all_dirs"))));
                ui.separator();
                ui.menu_button(tr("m.background"), |ui| {
                    for (bg, key) in [(Background::Checker, "m.bg_checker"), (Background::Black, "m.bg_black"), (Background::White, "m.bg_white"), (Background::Gray, "m.bg_gray")]
                    {
                        ui.radio_value(&mut self.settings.background, bg, tr(key));
                    }
                    // Fond personnalisé : le choisir (ou cliquer sur sa couleur) l'active et ouvre le sélecteur de couleur dans une
                    // fenêtre à part (un sélecteur ouvert dans le menu se fermerait avec lui).
                    ui.horizontal(|ui| {
                        let radio = ui.radio_value(&mut self.settings.background, Background::Custom, tr("m.bg_custom"));
                        let [r, g, b] = self.settings.bg_custom;
                        let (swatch, sw) = ui.allocate_exact_size(vec2(24.0, 14.0), Sense::click());
                        ui.painter().rect_filled(swatch, 3.0, Color32::from_rgb(r, g, b));
                        ui.painter().rect_stroke(swatch, 3.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.fg_stroke.color), egui::StrokeKind::Inside);
                        if radio.clicked() || sw.clicked() {
                            self.settings.background = Background::Custom;
                            self.open_bg_picker();
                            ui.close();
                        }
                    });
                });
            });
            // Navigation : ordre des fichiers, molette et boutons de la souris.
            ui.menu_button(tr("m.navigation"), |ui| {
                if item(ui, tr("m.current_folder"), "Ctrl+L", true) {
                    self.open_folder_dialog();
                }
                ui.separator();
                ui.label(tr("m.sort"));
                let mut sort_changed = false;
                for (key, label) in [(SortKey::Name, "m.sort_name"), (SortKey::Size, "m.sort_size"), (SortKey::Modified, "m.sort_date"), (SortKey::Type, "m.sort_type")] {
                    sort_changed |= ui.radio_value(&mut self.settings.sort, key, tr(label)).changed();
                }
                sort_changed |= ui.checkbox(&mut self.settings.sort_desc, tr("m.sort_desc")).changed();
                if sort_changed {
                    self.apply_view_options();
                }
                ui.separator();
                ui.label(tr("m.wheel"));
                ui.radio_value(&mut self.settings.wheel_nav, false, tr("m.wheel_zoom"));
                ui.radio_value(&mut self.settings.wheel_nav, true, tr("m.wheel_nav"));
                ui.separator();
                ui.label(tr("m.xbuttons"));
                ui.radio_value(&mut self.settings.xbtn_zoom, false, tr("m.xb_nav"));
                ui.radio_value(&mut self.settings.xbtn_zoom, true, tr("m.xb_zoom"));
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
                ui.separator();
                if item(ui, tr("m.extract_pal"), "Ctrl+P", has_img) {
                    self.open_palette_dialog();
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
                // Associations de fichiers (indisponibles sous macOS : il faudrait un bundle d'application).
                let can = crate::assoc::is_supported();
                if item(ui, tr("m.assoc_fallout"), "", can) {
                    self.associate_files(&crate::assoc::FALLOUT);
                }
                if item(ui, tr("m.assoc_tactics"), "", can) {
                    self.associate_files(&crate::assoc::TACTICS);
                }
                if item(ui, tr("m.assoc_arcanum"), "", can) {
                    self.associate_files(&crate::assoc::ARCANUM);
                }
                ui.separator();
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

    // ------------------------------------------------------------------ arborescence

    /// Boutons d'accès rapide (8 au plus) : Bureau, Documents, Téléchargements (réservés) puis les favoris ;
    /// clic droit sur un favori : le retirer.
    fn favorites_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut open: Option<std::path::PathBuf> = None;
        let mut remove: Option<usize> = None;
        let cur = self.tree.selected_dir.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
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
        if let Some(i) = remove {
            self.settings.favorites.remove(i);
        }
        if let Some(p) = open {
            if crate::dat::stat(&p).is_some() || p.is_dir() {
                self.open_dir(ctx, &p, false);
                self.reveal_in_tree();
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
        let path = s.loaded.path.clone();
        // Un fichier d'archive .DAT peut être copié mais ni déplacé ni renommé.
        let on_disk = !crate::dat::is_virtual(&path);
        // Boutons du bas (icônes seules) : image et chemin ; copier, déplacer, renommer ; convertir ; copier les informations.
        let mut action: Option<PropAction> = None;
        let sha = s.loaded.sha256.clone();
        let ref_hash = self.ref_hash.clone();
        egui::Panel::bottom("props_copy").show(ui, |ui| {
            ui.add_space(3.0);
            // Comparaison avec la dernière empreinte copiée ; la croix arrête le mode comparaison.
            if let Some((name, h)) = &ref_hash {
                // Ligne de hauteur fixe (celle du bouton) : le texte et la croix sont centrés verticalement dans la même bande.
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (color, key) = if *h == sha { (Color32::from_rgb(90, 190, 120), "t.sha_same") } else { (Color32::from_rgb(230, 120, 90), "t.sha_diff") };
                    ui.colored_label(color, i18n::trf(key, &[("name", name.clone())]));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Close, &tr("t.sha_stop"), false, true).clicked() {
                            action = Some(PropAction::ShaStop);
                        }
                    });
                });
                ui.separator();
            }
            // Pas d'espacement vertical après la dernière ligne : la marge du bas égale celle du haut.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal_wrapped(|ui| {
                    let mut button = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool, act: PropAction| {
                        if icon_button(ui, icon, &tr(tip), false, enabled).clicked() {
                            action = Some(act);
                        }
                    };
                    button(ui, Icon::CopyImage, "m.copy_image", true, PropAction::CopyImage);
                    button(ui, Icon::CopyPath, "t.copy_path", true, PropAction::CopyPath);
                    ui.separator();
                    button(ui, Icon::CopyTo, "t.copy_to", true, PropAction::CopyTo);
                    button(ui, Icon::MoveTo, "t.move_to", on_disk, PropAction::MoveTo);
                    button(ui, Icon::Rename, "t.rename", on_disk, PropAction::Rename);
                    ui.separator();
                    button(ui, Icon::Convert, "m.convert", true, PropAction::Convert);
                    button(ui, Icon::Palette, "m.extract_pal", true, PropAction::ConvertPalette);
                    ui.separator();
                    button(ui, Icon::CopyText, "t.copy_text", true, PropAction::Text);
                    button(ui, Icon::CopyJson, "t.copy_json", true, PropAction::Json);
                    button(ui, Icon::CopyXml, "t.copy_xml", true, PropAction::Xml);
                    button(ui, Icon::Sha, "t.copy_sha", true, PropAction::Sha);
                });
            });
            ui.add_space(3.0);
        });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (sid, sec) in pic.sections.iter().enumerate() {
                egui::CollapsingHeader::new(egui::RichText::new(tr(&sec.title)).strong()).id_salt(("sec", &sec.title)).default_open(sec.title != "sec.exif").show(ui, |ui| {
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
        let ctx = ui.ctx().clone();
        let info = match action {
            Some(PropAction::Text) => Some(props_text(&collect_props(pic, s.dir, s.frame))),
            Some(PropAction::Json) => Some(props_json(&collect_props(pic, s.dir, s.frame))),
            Some(PropAction::Xml) => Some(props_xml(&collect_props(pic, s.dir, s.frame))),
            _ => None,
        };
        if let Some(out) = info {
            ctx.copy_text(out);
            self.toast(tr("t.copied"));
        }
        match action {
            Some(PropAction::CopyImage) => self.copy_image(&ctx),
            Some(PropAction::CopyPath) => {
                ctx.copy_text(path.display().to_string());
                self.toast(tr("t.path_copied"));
            }
            Some(PropAction::Sha) => {
                ctx.copy_text(sha.clone());
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.ref_hash = Some((name, sha));
                self.toast(tr("t.sha_copied"));
            }
            Some(PropAction::ShaStop) => self.ref_hash = None,
            Some(PropAction::CopyTo) => self.copy_or_move_to(&ctx, &path, false),
            Some(PropAction::MoveTo) => self.copy_or_move_to(&ctx, &path, true),
            Some(PropAction::Rename) => self.open_rename(&path),
            Some(PropAction::Convert) => self.open_convert(),
            Some(PropAction::ConvertPalette) => self.open_palette_dialog(),
            _ => {}
        }
    }

    /// Demande un dossier puis copie ou déplace le fichier affiché ; après un déplacement, le fichier suivant s'affiche.
    pub fn copy_or_move_to(&mut self, ctx: &egui::Context, path: &std::path::Path, mv: bool) {
        let mut d = rfd::FileDialog::new().set_title(tr(if mv { "t.move_to" } else { "t.copy_to" }));
        if let Some(dir) = crate::dat::real_parent(path) {
            d = d.set_directory(dir);
        }
        let Some(dest) = d.pick_folder() else { return };
        let result = if mv { crate::fileops::move_to(path, &dest) } else { crate::fileops::copy_to(path, &dest) };
        match result {
            Ok(new) => {
                if mv {
                    let idx = self.index.unwrap_or(0);
                    // Rien à réafficher de force : le fichier déplacé n'est plus dans le dossier.
                    self.wanted = None;
                    self.reload_dir(ctx);
                    match self.files.get(idx.min(self.files.len().saturating_sub(1))).cloned() {
                        Some(next) => self.show_file(ctx, &next),
                        None => {
                            self.shown = None;
                            self.error = None;
                        }
                    }
                }
                self.toast(i18n::trf(if mv { "t.moved_to" } else { "t.copied_to" }, &[("path", new.display().to_string())]));
            }
            Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
        }
    }

    pub fn open_rename(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.rename_dlg = Some(crate::app::RenameDlg { path: path.to_path_buf(), name, fresh: true });
    }

    /// Fenêtre modale « Renommer » : le nom sans l'extension est sélectionné ; un nom invalide ou déjà pris est refusé.
    pub fn rename_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.rename_dlg.take() else { return };
        let name = dlg.name.trim().to_string();
        let current = dlg.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let invalid = crate::fileops::validate_name(&name).is_err();
        let exists = !invalid && !name.eq_ignore_ascii_case(&current) && dlg.path.with_file_name(&name).exists();
        let can_rename = !invalid && !exists && name != current;
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("rename_dlg")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(480.0);
            crate::dialog::setup(ui);
            cancel |= crate::dialog::title(ui, &tr("t.rename"));
            ui.add_space(4.0);
            let mut out = egui::TextEdit::singleline(&mut dlg.name).desired_width(f32::INFINITY).show(ui);
            if dlg.fresh {
                out.response.request_focus();
                let stem_end = dlg.name.rfind('.').filter(|i| *i > 0).unwrap_or(dlg.name.len());
                let chars = dlg.name[..stem_end].chars().count();
                out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars))));
                out.state.store(ui.ctx(), out.response.id);
            }
            if out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_rename {
                go = true;
            }
            if exists {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_exists"));
            } else if invalid && !dlg.name.trim().is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_invalid"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                go |= crate::dialog::button_enabled(ui, can_rename, tr("c.ok")).clicked();
                cancel |= crate::dialog::button(ui, tr("c.cancel")).clicked();
            });
        });
        if go {
            match crate::fileops::rename(&dlg.path, &dlg.name) {
                Ok(new) => {
                    self.toast(i18n::trf("t.renamed", &[("name", new.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]));
                    self.wanted = Some(new);
                    self.reload_dir(ctx);
                }
                Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
            }
        } else if !cancel && !modal.should_close() {
            dlg.fresh = false;
            self.rename_dlg = Some(dlg);
        }
    }

    pub fn open_anim_dialog(&mut self, dir: &std::path::Path) {
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "animation".into());
        self.anim_dlg = Some(crate::app::AnimDlg { src: dir.to_path_buf(), format: Format::Frm, name, out_dir: dir.display().to_string(), fresh: true });
    }

    pub fn anim_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.anim_dlg.take() else { return };
        let name = dlg.name.trim().to_string();
        let invalid = crate::fileops::validate_name(&name).is_err();
        let out_dir = std::path::PathBuf::from(dlg.out_dir.trim());
        let target = crate::batch::animation_path(&out_dir, &name, dlg.format);
        let exists = !invalid && target.exists();
        let can_create = !invalid && !exists && !dlg.out_dir.trim().is_empty();
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("anim_dlg")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            crate::dialog::setup(ui);
            cancel |= crate::dialog::title(ui, &tr("t.create_anim"));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(dlg.src.display().to_string()).weak());
            ui.add_space(4.0);
            egui::Grid::new("anim_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label(tr("t.anim_format"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut dlg.format, Format::Frm, "FRM");
                    ui.selectable_value(&mut dlg.format, Format::Art, "ART");
                });
                ui.end_row();
                ui.label(tr("t.anim_name"));
                let edit = ui.add(egui::TextEdit::singleline(&mut dlg.name).desired_width(f32::INFINITY));
                if dlg.fresh {
                    edit.request_focus();
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_create {
                    go = true;
                }
                ui.end_row();
                ui.label(tr("t.anim_outdir"));
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 36.0;
                    ui.add(egui::TextEdit::singleline(&mut dlg.out_dir).desired_width(w));
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new().set_title(tr("t.anim_outdir")).set_directory(&out_dir).pick_folder()
                    {
                        dlg.out_dir = p.display().to_string();
                    }
                });
                ui.end_row();
            });
            if exists {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_exists"));
            } else if invalid && !dlg.name.trim().is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_invalid"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                go |= crate::dialog::button_enabled(ui, can_create, tr("c.ok")).clicked();
                cancel |= crate::dialog::button(ui, tr("c.cancel")).clicked();
            });
        });
        if go {
            let (src, fmt, name) = (dlg.src.clone(), dlg.format, dlg.name.clone());
            self.create_animation(ctx, &src, &out_dir, &name, fmt);
        } else if !cancel && !modal.should_close() {
            dlg.fresh = false;
            self.anim_dlg = Some(dlg);
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
        let mut nav_step = 0i32;

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
                if self.fit && (self.zoom - 1.0).abs() > 1e-3 {
                    self.zoom_actual();
                } else if self.fit {
                    // déjà à 100 %
                } else {
                    self.fit = true;
                    self.fit_up = true;
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
            let anchors: Vec<(f32, f32)> = if self.settings.show_origin && has_anchor {
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
                        for st in [dark, light] {
                            painter.circle_stroke(c, 6.0, st);
                            painter.line_segment([c - vec2(14.0, 0.0), c + vec2(14.0, 0.0)], st);
                            painter.line_segment([c - vec2(0.0, 14.0), c + vec2(0.0, 14.0)], st);
                        }
                    }
                }
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
                // Pixel sous le curseur : position, couleur, indice de palette.
                if self.settings.show_pixel_info
                    && let Some(hp) = resp.hover_pos()
                    && img_rect.contains(hp)
                {
                    let tx = (((hp.x - img_rect.min.x) / sc).floor().max(0.0) as usize).min((w as usize).saturating_sub(1));
                    let ty = (((hp.y - img_rect.min.y) / sc).floor().max(0.0) as usize).min((h as usize).saturating_sub(1));
                    let (sx, sy) = self.transform.source_of(tx, ty, cw0 as usize, ch0 as usize);
                    let info = self.shown.as_ref().and_then(|s| match &grid {
                        Some(g) => g.cell_at(sx as i32, sy as i32).and_then(|(d, lx, ly)| {
                            let pic = &s.loaded.pic;
                            let b = pic.dir_bounds(d);
                            pixel_info(pic, d, s.frame.min(pic.dir_len(d).saturating_sub(1)), lx - (b.0 - g.x0), ly - (b.1 - g.y0))
                        }),
                        None => pixel_info(&s.loaded.pic, s.dir, s.frame, sx as i32, sy as i32),
                    });
                    if let Some(info) = info {
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

    // ------------------------------------------------------------------ fenêtres annexes

    pub fn cli_help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_cli_help;
        egui::Window::new(tr("m.cli_help")).open(&mut open).frame(crate::dialog::window_frame(ctx)).collapsible(false).resizable(true).default_size([760.0, 560.0]).show(
            ctx,
            |ui| {
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
            },
        );
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
        let modal = egui::Modal::new(Id::new("folder_dlg")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(560.0);
            crate::dialog::setup(ui);
            cancel |= crate::dialog::title(ui, &tr("t.folder"));
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
            ui.separator();
            ui.horizontal(|ui| {
                go |= crate::dialog::button_enabled(ui, valid, tr("c.ok")).clicked();
                cancel |= crate::dialog::button(ui, tr("c.cancel")).clicked();
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
        let modal = egui::Modal::new(Id::new("expand_confirm")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(420.0);
            crate::dialog::setup(ui);
            no |= crate::dialog::title(ui, &tr("t.expand_all"));
            ui.add(egui::Label::new(tr("t.expand_confirm")).wrap());
            ui.separator();
            ui.horizontal(|ui| {
                yes = crate::dialog::button(ui, tr("t.expand_all")).clicked();
                no |= crate::dialog::button(ui, tr("c.cancel")).clicked();
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
        egui::Window::new(tr("m.about"))
            .open(&mut open)
            .frame(crate::dialog::window_frame(ctx))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
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

/// Curseur à dégradé : `value` entre 0 et 1, `track(t)` donne la couleur du dégradé en `t`. Renvoie vrai si la valeur change.
/// Première lettre ou chiffre d'un nom (en majuscule), pour le bouton d'un favori.
fn letter_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).or_else(|| name.chars().next()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_string())
}

/// Bouton carré affichant une lettre.
fn letter_button(ui: &mut Ui, letter: &str, tip: &str, selected: bool) -> egui::Response {
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

fn gradient_slider(ui: &mut Ui, value: &mut f32, width: f32, track: impl Fn(f32) -> Color32) -> bool {
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
}
