//! Barre de menus.

use crate::app::{App, OriginStyle, Smoothing, ThemeChoice};
use crate::i18n::{self, Lang, tr};
use crate::loader::PalPolicy;
use crate::model::*;
use crate::tree::SortKey;
use crate::view::Background;
use crate::view::can_show_all_dirs;
use eframe::egui::{self, Color32, Sense, Stroke, Ui, Vec2, vec2};

impl App {
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
                if item(ui, tr("m.edit_frm"), "", is_frm) {
                    self.open_frm_editor();
                }
                ui.separator();
                if item(ui, tr("m.batch"), "Ctrl+B", true) {
                    self.open_batch();
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
                ui.checkbox(&mut self.settings.show_colors, tr("colors"));
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
                ui.menu_button(format!("{}  [A]", tr("m.origin")), |ui| {
                    for st in OriginStyle::ALL {
                        if ui.radio_value(&mut self.settings.origin, st, tr(st.key())).clicked() {
                            ui.close();
                        }
                    }
                });
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
                ui.separator();
                ui.checkbox(&mut self.settings.taskbar_windows, tr("m.taskbar_windows")).on_hover_text(tr("m.taskbar_windows_tip"));
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
            // Paramètres : langue et thème (système, sombre ou clair).
            ui.menu_button(tr("m.settings"), |ui| {
                ui.menu_button(tr("m.language"), |ui| {
                    for l in Lang::ALL {
                        if ui.radio(i18n::lang() == l, l.native_name()).clicked() {
                            self.set_language(&ctx, l);
                            ui.close();
                        }
                    }
                });
                ui.menu_button(tr("m.theme"), |ui| {
                    for t in [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light] {
                        if ui.radio(self.settings.theme == t, tr(t.key())).clicked() {
                            self.settings.theme = t;
                            ctx.set_theme(t.preference());
                            ui.close();
                        }
                    }
                });
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
}
