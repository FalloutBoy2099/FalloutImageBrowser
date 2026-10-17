//! Entrées clavier et souris, saisie au clavier dans l'arborescence.

use crate::app::App;
use crate::loader::PalPolicy;
use crate::tree::RowKind;
use eframe::egui::{self, Vec2};
use std::time::{Duration, Instant};

impl App {
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
}
