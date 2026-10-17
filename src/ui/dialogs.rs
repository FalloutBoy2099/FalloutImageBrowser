//! Fenêtres annexes : renommer, animation, aide, dossier courant, confirmation, À propos et notification.

use crate::app::App;
use crate::i18n::{self, tr};
use crate::model::*;
use eframe::egui::{self, Align2, Id, vec2};
use std::time::Duration;

use super::extra_window;

impl App {
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

    pub fn cli_help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_cli_help;
        extra_window(ctx, self.settings.taskbar_windows, "cli_help_window", tr("m.cli_help"), [760.0, 560.0], true, false, &mut open, |ui| {
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
        extra_window(ctx, self.settings.taskbar_windows, "about_window", tr("m.about"), [540.0, 520.0], false, true, &mut open, |ui| {
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
