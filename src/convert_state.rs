//! État de la fenêtre de conversion : réglages en cours, estimation de la taille du fichier produit.

use crate::convert::*;
use crate::formats::{self, EncodeOpts, PngLevel};
use crate::i18n::tr;
use crate::imageops::{DitherMethod, ImageOps, Resample, Resize};
use crate::loader::{Loaded, load_pal_file};
use crate::model::*;
use crate::convert_settings::*;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Résultat de l'estimation : (octets, largeur, hauteur) de l'image produite.
type EstimateResult = Result<(usize, u32, u32), String>;

/// Taille estimée du fichier produit : la conversion est réellement faite, en arrière-plan, sur l'image affichée.
#[derive(Default)]
pub struct Estimate {
    /// Signature des réglages pour lesquels `value` a été calculée.
    sig: String,
    changed: Option<Instant>,
    running: bool,
    /// Le calcul a été demandé à la main (grandes images).
    manual: bool,
    done: Arc<Mutex<Option<(String, EstimateResult)>>>,
    value: Option<EstimateResult>,
}

/// Au-delà de ce nombre de pixels, l'estimation n'est calculée que sur demande (elle peut prendre plusieurs secondes).
const AUTO_ESTIMATE_PIXELS: u64 = 40_000_000;

/// Délai entre un changement de réglage et le début du calcul de l'estimation.
const ESTIMATE_DELAY: Duration = Duration::from_millis(400);

/// « 1,2 Mo » : taille courte pour l'estimation.
pub fn short_size(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", n as f64 / 1048576.0)
    }
}

pub struct ConvertUi {
    pub open: bool,
    pub s: ConvSettings,
    pub custom: Option<(Arc<Palette>, String)>,
    pub name: String,
    pub source: Option<(Arc<Loaded>, Selection)>,
    pub running: bool,
    pub outcome: Outcome,
    pub error: Option<String>,
    pub estimate: Estimate,
}

impl ConvertUi {
    pub fn new(s: &ConvSettings) -> ConvertUi {
        let custom = s.pal_path.as_ref().and_then(|p| load_pal_file(p).ok().map(|pal| (Arc::new(pal), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())));
        ConvertUi {
            open: false,
            s: s.clone(),
            custom,
            name: String::new(),
            source: None,
            running: false,
            outcome: Arc::new(Mutex::new(None)),
            error: None,
            estimate: Estimate::default(),
        }
    }

    pub fn to_settings(&self) -> ConvSettings {
        self.s.clone()
    }

    pub fn format(&self) -> Format {
        Format::from_ext(&self.s.fmt).unwrap_or(Format::Bmp)
    }

    pub fn depth_choice(&self) -> Option<DepthChoice> {
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

    /// Options de conversion correspondant aux réglages (`None` : une palette personnalisée reste à choisir).
    pub fn options(&self) -> Option<ConvertOptions> {
        Some(ConvertOptions {
            format: self.format(),
            depth: self.depth_choice()?,
            dither: self.s.dither,
            bg: self.s.bg,
            enc: EncodeOpts { jpeg_quality: self.s.jpeg_quality, png_level: self.s.png_level, ktx2_bc7: self.s.ktx2_bc7 },
            ops: ImageOps { resize: self.s.resize, resample: self.s.resample, trim: self.s.trim, dither_method: self.s.dither_method },
            text_white: self.s.text_white,
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

    pub fn fix_depth(&mut self) {
        if !self.supported(self.s.depth)
            && let Some(k) = DepthKind::ALL.iter().copied().find(|k| self.supported(*k))
        {
            self.s.depth = k;
        }
    }

    /// Dossier proposé : le dernier utilisé, sinon celui du fichier source.
    pub(crate) fn source_dir(&self) -> Option<PathBuf> {
        self.source.as_ref().and_then(|(l, _)| crate::dat::real_parent(&l.path))
    }

    pub(crate) fn effective_out_dir(&self) -> Option<PathBuf> {
        self.s.out_dir.clone().or_else(|| self.s.last_out_dir.clone()).or_else(|| self.source_dir())
    }

    /// Lignes de réglages communes à la conversion d'une image et au traitement par lot : format, profondeur, tramage,
    /// qualité, taille, marges, fond, animations. À dessiner dans une grille à deux colonnes.
    pub fn options_rows(&mut self, ui: &mut egui::Ui, animations: bool) {
        ui.label(tr("c.format"));
        let before = self.s.fmt.clone();
        egui::ComboBox::from_id_salt("fmt").selected_text(format!("{} (.{})", self.format().name(), self.format().ext())).show_ui(ui, |ui| {
            for f in Format::ALL.iter() {
                ui.selectable_value(&mut self.s.fmt, f.ext().to_string(), format!("{} (.{})", f.name(), f.ext()));
            }
        });
        if before != self.s.fmt {
            self.fix_depth();
        }
        ui.end_row();

        ui.label(tr("c.depth"));
        egui::ComboBox::from_id_salt("depth").selected_text(tr(self.s.depth.key())).show_ui(ui, |ui| {
            for k in DepthKind::ALL {
                if self.supported(k) {
                    ui.selectable_value(&mut self.s.depth, k, tr(k.key()));
                }
            }
        });
        ui.end_row();

        if self.s.depth == DepthKind::Custom8 {
            ui.label(tr("c.pal_file"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.s.create_pal, tr("c.pal_create"));
                if self.s.create_pal {
                    ui.checkbox(&mut self.s.text_white, tr("c.pal_white220")).on_hover_text(tr("c.pal_white220_tip"));
                }
                ui.add_enabled_ui(!self.s.create_pal, |ui| {
                    if ui.button(tr("c.browse")).clicked()
                        && let Some(p) = rfd::FileDialog::new().add_filter("PAL", &["pal", "PAL"]).pick_file()
                    {
                        match load_pal_file(&p) {
                            Ok(pal) => {
                                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                self.s.pal_path = Some(p);
                                self.custom = Some((Arc::new(pal), name));
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                    match &self.custom {
                        Some((_, n)) => ui.label(n.clone()),
                        None => ui.label(egui::RichText::new(tr("c.none")).weak()),
                    };
                });
            });
            ui.end_row();
        }
        if matches!(self.s.depth, DepthKind::Pal8 | DepthKind::Fallout8 | DepthKind::Custom8) {
            ui.label(tr("c.dither"));
            let mut sel = match (self.s.dither, self.s.dither_method) {
                (false, _) => 0,
                (true, DitherMethod::FloydSteinberg) => 1,
                (true, DitherMethod::Bayer) => 2,
            };
            const KEYS: [&str; 3] = ["c.dither_none", "c.dither_fs", "c.dither_bayer"];
            egui::ComboBox::from_id_salt("dither").selected_text(tr(KEYS[sel])).show_ui(ui, |ui| {
                for (i, k) in KEYS.iter().enumerate() {
                    ui.selectable_value(&mut sel, i, tr(k));
                }
            });
            self.s.dither = sel != 0;
            if sel != 0 {
                self.s.dither_method = if sel == 2 { DitherMethod::Bayer } else { DitherMethod::FloydSteinberg };
            }
            ui.end_row();
        }
        if self.format() == Format::Jpeg {
            ui.label(tr("c.quality"));
            ui.add(egui::Slider::new(&mut self.s.jpeg_quality, 1..=100));
            ui.end_row();
        }
        if self.format() == Format::Png {
            ui.label(tr("c.png_level"));
            egui::ComboBox::from_id_salt("png_level").selected_text(tr(self.s.png_level.key())).show_ui(ui, |ui| {
                for l in PngLevel::ALL {
                    ui.selectable_value(&mut self.s.png_level, l, tr(l.key()));
                }
            });
            ui.end_row();
        }
        if self.format() == Format::Ktx2 {
            ui.label(tr("c.ktx2_mode"));
            egui::ComboBox::from_id_salt("ktx2_mode").selected_text(tr(if self.s.ktx2_bc7 { "c.ktx2_bc7" } else { "c.ktx2_lossless" })).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.s.ktx2_bc7, false, tr("c.ktx2_lossless"));
                ui.selectable_value(&mut self.s.ktx2_bc7, true, tr("c.ktx2_bc7"));
            });
            ui.end_row();
        }
        // Taille : originale, pourcentage ou plus grand côté.
        ui.label(tr("c.resize"));
        ui.horizontal(|ui| {
            let mut kind = match self.s.resize {
                Resize::None => 0,
                Resize::Percent(_) => 1,
                Resize::MaxSide(_) => 2,
            };
            const KEYS: [&str; 3] = ["c.rs_none", "c.rs_percent", "c.rs_max"];
            let before = kind;
            egui::ComboBox::from_id_salt("resize").selected_text(tr(KEYS[kind])).show_ui(ui, |ui| {
                for (i, k) in KEYS.iter().enumerate() {
                    ui.selectable_value(&mut kind, i, tr(k));
                }
            });
            if kind != before {
                self.s.resize = [Resize::None, Resize::Percent(50), Resize::MaxSide(1024)][kind];
            }
            match &mut self.s.resize {
                Resize::Percent(p) => {
                    ui.add(egui::DragValue::new(p).range(1..=1000).suffix(" %"));
                }
                Resize::MaxSide(n) => {
                    ui.add(egui::DragValue::new(n).range(1..=crate::imageops::MAX_SIDE as u32).suffix(" px"));
                }
                Resize::None => {}
            }
        });
        ui.end_row();
        if self.s.resize != Resize::None {
            ui.label(tr("c.resample"));
            egui::ComboBox::from_id_salt("resample").selected_text(tr(if self.s.resample == Resample::Smooth { "c.rs_smooth" } else { "c.rs_pixel" })).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.s.resample, Resample::Smooth, tr("c.rs_smooth"));
                ui.selectable_value(&mut self.s.resample, Resample::Pixel, tr("c.rs_pixel"));
            });
            ui.end_row();
        }
        ui.label(tr("c.trim"));
        ui.checkbox(&mut self.s.trim, "");
        ui.end_row();
        ui.label(tr("c.bg"));
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut self.s.bg);
            ui.label(egui::RichText::new(tr("c.bg_hint")).weak());
        });
        ui.end_row();
        if animations && !formats::caps(self.format()).multi_frame {
            ui.label(tr("c.frames"));
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.s.sequence, false, tr("c.fr_current"));
                ui.radio_value(&mut self.s.sequence, true, tr("c.fr_all"));
            });
            ui.end_row();
        }
    }

    /// Signature des réglages qui influent sur la taille du fichier produit.
    fn signature(&self) -> String {
        let s = &self.s;
        format!(
            "{}|{:?}|{}|{:?}|{:?}|{:?}|{}|{:?}|{:?}|{}|{}|{:?}|{}|{}|{:?}",
            s.fmt,
            s.depth,
            s.dither,
            s.dither_method,
            s.resize,
            s.resample,
            s.trim,
            s.bg,
            s.png_level,
            s.ktx2_bc7,
            s.jpeg_quality,
            s.pal_path,
            s.create_pal,
            s.text_white,
            self.source.as_ref().map(|(l, sel)| (l.path.clone(), sel.dir, sel.frame))
        )
    }

    /// Lance (ou relance) le calcul de l'estimation quand les réglages ont changé depuis un instant, et récupère son résultat.
    pub(crate) fn update_estimate(&mut self, ctx: &egui::Context) {
        let Some((loaded, sel)) = self.source.clone() else { return };
        let sig = self.signature();
        if let Some((done_sig, r)) = self.estimate.done.lock().unwrap().take() {
            self.estimate.running = false;
            if done_sig == sig {
                self.estimate.value = Some(r);
            }
        }
        if self.estimate.sig != sig {
            self.estimate.sig = sig.clone();
            self.estimate.value = None;
            self.estimate.changed = Some(Instant::now());
            self.estimate.manual = false;
        }
        if self.estimate.value.is_some() || self.estimate.running {
            return;
        }
        let pixels: u64 = loaded.pic.frames.iter().map(|f| f.width as u64 * f.height as u64).sum();
        if pixels > AUTO_ESTIMATE_PIXELS && !self.estimate.manual {
            return;
        }
        let Some(opts) = self.options() else { return };
        let wait = self.estimate.changed.map_or(Duration::ZERO, |t| ESTIMATE_DELAY.saturating_sub(t.elapsed()));
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
            return;
        }
        self.estimate.running = true;
        let done = self.estimate.done.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> EstimateResult {
                let prep = prepare(&loaded.pic, &opts, &sel).map_err(|e| e.to_string())?;
                let (w, h) = (prep.first().width, prep.first().height);
                let bytes = formats::encode(&prep, &opts.enc).map_err(|e| e.to_string())?;
                Ok((bytes.len(), w, h))
            }))
            .unwrap_or_else(|_| Err("erreur interne".into()));
            *done.lock().unwrap() = Some((sig, r));
            ctx.request_repaint();
        });
    }

    /// Ligne « Résultat » : dimensions et taille estimée du fichier.
    pub(crate) fn estimate_row(&mut self, ui: &mut egui::Ui) {
        ui.label(tr("c.result"));
        ui.horizontal(|ui| match &self.estimate.value {
            Some(Ok((bytes, w, h))) => {
                ui.label(format!("{w} × {h}   ≈ {}", short_size(*bytes)));
            }
            Some(Err(e)) => {
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            None if self.estimate.running => {
                ui.spinner();
                ui.label(egui::RichText::new(tr("c.est_calc")).weak());
            }
            None => {
                let big = self.source.as_ref().is_some_and(|(l, _)| l.pic.frames.iter().map(|f| f.width as u64 * f.height as u64).sum::<u64>() > AUTO_ESTIMATE_PIXELS);
                if big && ui.button(tr("c.est_btn")).clicked() {
                    self.estimate.manual = true;
                }
            }
        });
        ui.end_row();
    }
}

