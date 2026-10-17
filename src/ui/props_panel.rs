//! Panneau « Propriétés ».

use crate::app::App;
use crate::i18n::{self, tr};
use crate::icons::{Icon, icon_button};
use crate::props::{collect_props, props_json, props_text, props_xml};
use eframe::egui::{self, Color32, Ui, vec2};

use super::widgets::palette_grid;

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

impl App {
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
}
