//! Fenêtre modale de conversion de l'image affichée.

use crate::app::App;
use crate::batch::{self, Job};
use crate::convert::*;
use crate::formats::{self, EncodeOpts};
use crate::i18n::{tr, trf};
use crate::loader::{Loaded, load_pal_file};
use crate::model::*;
use eframe::egui::{self, Align2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum DepthKind {
    Pal8,
    Fallout8,
    Custom8,
    Rgb16,
    Rgb24,
    Rgba32,
}

impl DepthKind {
    pub const ALL: [DepthKind; 6] = [DepthKind::Pal8, DepthKind::Fallout8, DepthKind::Custom8, DepthKind::Rgb16, DepthKind::Rgb24, DepthKind::Rgba32];

    pub fn key(self) -> &'static str {
        match self {
            DepthKind::Pal8 => "c.d8",
            DepthKind::Fallout8 => "c.d8f",
            DepthKind::Custom8 => "c.d8p",
            DepthKind::Rgb16 => "c.d16",
            DepthKind::Rgb24 => "c.d24",
            DepthKind::Rgba32 => "c.d32",
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ConvSettings {
    /// Extension du format cible (stable même si la liste des formats évolue).
    pub fmt: String,
    pub depth: DepthKind,
    pub dither: bool,
    pub bg: [u8; 3],
    pub jpeg_quality: u8,
    pub sequence: bool,
    /// Dernier dossier de sortie utilisé.
    pub out_dir: Option<PathBuf>,
    pub pal_path: Option<PathBuf>,
    /// Dernier dossier de sortie effectivement utilisé par une conversion.
    pub last_out_dir: Option<PathBuf>,
    /// Palette personnalisée : créer le fichier .PAL à partir de l'image (sinon choisir un .PAL existant).
    pub create_pal: bool,
}

impl Default for ConvSettings {
    fn default() -> Self {
        ConvSettings {
            fmt: "bmp".to_string(),
            depth: DepthKind::Rgb24,
            dither: false,
            bg: [0, 0, 0],
            jpeg_quality: 92,
            sequence: false,
            out_dir: None,
            pal_path: None,
            last_out_dir: None,
            create_pal: true,
        }
    }
}

type Outcome = Arc<Mutex<Option<Result<Vec<PathBuf>, String>>>>;

pub struct ConvertUi {
    pub open: bool,
    pub s: ConvSettings,
    pub custom: Option<(Arc<Palette>, String)>,
    pub name: String,
    pub source: Option<(Arc<Loaded>, Selection)>,
    pub running: bool,
    pub outcome: Outcome,
    pub error: Option<String>,
}

impl ConvertUi {
    pub fn new(s: &ConvSettings) -> ConvertUi {
        let custom = s.pal_path.as_ref().and_then(|p| {
            load_pal_file(p).ok().map(|pal| (Arc::new(pal), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
        });
        ConvertUi {
            open: false,
            s: s.clone(),
            custom,
            name: String::new(),
            source: None,
            running: false,
            outcome: Arc::new(Mutex::new(None)),
            error: None,
        }
    }

    pub fn to_settings(&self) -> ConvSettings {
        self.s.clone()
    }

    fn format(&self) -> Format {
        Format::from_ext(&self.s.fmt).unwrap_or(Format::Bmp)
    }

    fn depth_choice(&self) -> Option<DepthChoice> {
        Some(match self.s.depth {
            DepthKind::Pal8 => DepthChoice::Indexed(PalChoice::Adaptive),
            DepthKind::Fallout8 => DepthChoice::Indexed(PalChoice::Fallout),
            DepthKind::Custom8 if self.s.create_pal => DepthChoice::Indexed(PalChoice::Create),
            DepthKind::Custom8 => DepthChoice::Indexed(PalChoice::Custom(self.custom.as_ref()?.0.clone())),
            DepthKind::Rgb16 => DepthChoice::Rgb16,
            DepthKind::Rgb24 => DepthChoice::Rgb24,
            DepthKind::Rgba32 => DepthChoice::Rgba32,
        })
    }

    fn supported(&self, k: DepthKind) -> bool {
        let probe = match k {
            DepthKind::Pal8 => DepthChoice::Indexed(PalChoice::Adaptive),
            DepthKind::Fallout8 => DepthChoice::Indexed(PalChoice::Fallout),
            DepthKind::Custom8 => DepthChoice::Indexed(PalChoice::Create),
            DepthKind::Rgb16 => DepthChoice::Rgb16,
            DepthKind::Rgb24 => DepthChoice::Rgb24,
            DepthKind::Rgba32 => DepthChoice::Rgba32,
        };
        depth_supported(&formats::caps(self.format()), &probe)
    }

    fn fix_depth(&mut self) {
        if !self.supported(self.s.depth) {
            if let Some(k) = DepthKind::ALL.iter().copied().find(|k| self.supported(*k)) {
                self.s.depth = k;
            }
        }
    }

    /// Dossier proposé : le dernier utilisé, sinon celui du fichier source.
    fn source_dir(&self) -> Option<PathBuf> {
        self.source.as_ref().and_then(|(l, _)| crate::dat::real_parent(&l.path))
    }

    fn effective_out_dir(&self) -> Option<PathBuf> {
        self.s.out_dir.clone().or_else(|| self.s.last_out_dir.clone()).or_else(|| self.source_dir())
    }
}

impl App {
    pub fn open_convert(&mut self) {
        let Some(s) = &self.shown else { return };
        self.conv.source = Some((s.loaded.clone(), Selection { dir: s.dir, frame: s.frame }));
        self.conv.name = batch::stem_of(&s.loaded.path);
        self.conv.error = None;
        self.conv.fix_depth();
        self.conv.open = true;
    }

    fn start_conversion(&mut self, ctx: &egui::Context) {
        let Some(depth) = self.conv.depth_choice() else {
            self.conv.error = Some(tr("c.need_pal"));
            return;
        };
        let (Some((loaded, sel)), Some(out_dir)) = (self.conv.source.clone(), self.conv.effective_out_dir()) else { return };
        let job = Job {
            opts: ConvertOptions {
                format: self.conv.format(),
                depth,
                dither: self.conv.s.dither,
                bg: self.conv.s.bg,
                enc: EncodeOpts { jpeg_quality: self.conv.s.jpeg_quality },
            },
            seq: self.conv.s.sequence,
            policy: self.policy.clone(),
        };
        let name = self.conv.name.clone();
        self.conv.error = None;
        self.conv.running = true;
        *self.conv.outcome.lock().unwrap() = None;
        let outcome = self.conv.outcome.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| batch::convert_loaded(&loaded, &sel, &job, &out_dir, Some(&name))))
                .unwrap_or_else(|_| Err("erreur interne".into()));
            *outcome.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
    }

    fn poll_conversion(&mut self) {
        if !self.conv.running {
            return;
        }
        let done = self.conv.outcome.lock().unwrap().take();
        let Some(res) = done else { return };
        self.conv.running = false;
        match res {
            Ok(files) => {
                let out_dir = self.conv.effective_out_dir();
                self.conv.s.out_dir = out_dir.clone();
                self.conv.s.last_out_dir = out_dir.clone();
                let what = match files.len() {
                    1 => files[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    n => format!("{n}"),
                };
                self.conv.open = false;
                self.toast(trf("c.done", &[("file", what)]));
                // Rafraîchir la liste si on a écrit dans le dossier courant.
                if let (Some(dir), Some(cur)) = (self.tree.selected_dir.clone(), out_dir) {
                    if dir == cur {
                        self.tree.reload_dir(&dir);
                        self.files = self.tree.files_of(&dir);
                        if let Some(w) = &self.wanted {
                            self.index = self.files.iter().position(|f| f == w);
                        }
                    }
                }
            }
            Err(e) => self.conv.error = Some(e),
        }
    }

    pub fn convert_window(&mut self, ctx: &egui::Context) {
        self.poll_conversion();
        if !self.conv.open {
            return;
        }
        let mut start = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("convert_modal")).show(ctx, |ui| {
            ui.set_width(470.0);
            ui.heading(tr("c.title"));
            if let Some((l, _)) = &self.conv.source {
                ui.label(egui::RichText::new(l.path.display().to_string()).weak().small());
            }
            ui.add_space(6.0);
            let running = self.conv.running;
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("conv_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(tr("c.format"));
                    let before = self.conv.s.fmt.clone();
                    egui::ComboBox::from_id_salt("fmt").selected_text(format!("{} (.{})", self.conv.format().name(), self.conv.format().ext())).show_ui(ui, |ui| {
                        for f in Format::ALL.iter() {
                            ui.selectable_value(&mut self.conv.s.fmt, f.ext().to_string(), format!("{} (.{})", f.name(), f.ext()));
                        }
                    });
                    if before != self.conv.s.fmt {
                        self.conv.fix_depth();
                    }
                    ui.end_row();

                    ui.label(tr("c.depth"));
                    egui::ComboBox::from_id_salt("depth").selected_text(tr(self.conv.s.depth.key())).show_ui(ui, |ui| {
                        for k in DepthKind::ALL {
                            if self.conv.supported(k) {
                                ui.selectable_value(&mut self.conv.s.depth, k, tr(k.key()));
                            }
                        }
                    });
                    ui.end_row();

                    if self.conv.s.depth == DepthKind::Custom8 {
                        ui.label(tr("c.pal_file"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.conv.s.create_pal, tr("c.pal_create"));
                            ui.add_enabled_ui(!self.conv.s.create_pal, |ui| {
                            if ui.button(tr("c.browse")).clicked() {
                                if let Some(p) = rfd::FileDialog::new().add_filter("PAL", &["pal", "PAL"]).pick_file() {
                                    match load_pal_file(&p) {
                                        Ok(pal) => {
                                            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                            self.conv.s.pal_path = Some(p);
                                            self.conv.custom = Some((Arc::new(pal), name));
                                        }
                                        Err(e) => self.conv.error = Some(e),
                                    }
                                }
                            }
                            match &self.conv.custom {
                                Some((_, n)) => ui.label(n.clone()),
                                None => ui.label(egui::RichText::new(tr("c.none")).weak()),
                            };
                            });
                        });
                        ui.end_row();
                    }
                    if matches!(self.conv.s.depth, DepthKind::Pal8 | DepthKind::Fallout8 | DepthKind::Custom8) {
                        ui.label(tr("c.dither"));
                        ui.checkbox(&mut self.conv.s.dither, "");
                        ui.end_row();
                    }
                    if self.conv.format() == Format::Jpeg {
                        ui.label(tr("c.quality"));
                        ui.add(egui::Slider::new(&mut self.conv.s.jpeg_quality, 1..=100));
                        ui.end_row();
                    }
                    ui.label(tr("c.bg"));
                    ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(&mut self.conv.s.bg);
                        ui.label(egui::RichText::new(tr("c.bg_hint")).weak().small());
                    });
                    ui.end_row();
                    let multi = self.conv.source.as_ref().is_some_and(|(l, _)| l.pic.frames.len() > 1);
                    if multi && !formats::caps(self.conv.format()).multi_frame {
                        ui.label(tr("c.frames"));
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.conv.s.sequence, false, tr("c.fr_current"));
                            ui.radio_value(&mut self.conv.s.sequence, true, tr("c.fr_all"));
                        });
                        ui.end_row();
                    }
                    ui.label(tr("c.name"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.conv.name).desired_width(240.0));
                        ui.label(format!(".{}", self.conv.format().ext()));
                    });
                    ui.end_row();
                    ui.label(tr("c.out_dir"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            if ui.button(tr("c.choose")).clicked() {
                                let mut d = rfd::FileDialog::new();
                                if let Some(cur) = self.conv.effective_out_dir() {
                                    d = d.set_directory(cur);
                                }
                                if let Some(p) = d.pick_folder() {
                                    self.conv.s.out_dir = Some(p);
                                }
                            }
                            if ui.button(tr("c.out_source")).clicked() {
                                self.conv.s.out_dir = self.conv.source_dir();
                            }
                            let last = self.conv.s.last_out_dir.clone();
                            if ui.add_enabled(last.is_some(), egui::Button::new(tr("c.out_last"))).clicked() {
                                self.conv.s.out_dir = last;
                            }
                        });
                        if let Some(p) = self.conv.effective_out_dir() {
                            ui.label(egui::RichText::new(p.display().to_string()).small());
                        }
                    });
                    ui.end_row();
                });
            });
            if let Some(e) = &self.conv.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let can = !self.conv.running && self.conv.effective_out_dir().is_some() && !self.conv.name.trim().is_empty();
                if ui.add_enabled(can, egui::Button::new(tr("c.start"))).clicked() {
                    start = true;
                }
                if ui.add_enabled(!self.conv.running, egui::Button::new(tr("c.cancel"))).clicked() {
                    close = true;
                }
                if self.conv.running {
                    ui.spinner();
                }
            });
        });
        if modal.should_close() && !self.conv.running {
            close = true;
        }
        if close {
            self.conv.open = false;
        }
        if start {
            self.start_conversion(ctx);
        }
        let _ = Align2::CENTER_CENTER;
    }
}
