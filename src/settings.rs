//! Réglages de l'utilisateur (enregistrés dans le magasin de préférences, voir `store`).

use crate::i18n::{self, Lang, tr};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SETTINGS_KEY: &str = "fib_settings_v1";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum Smoothing {
    #[default]
    Auto,
    On,
    Off,
}

/// Curseur des barres de défilement : taille d'egui, ou longueur minimale confortable.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum ScrollbarStyle {
    #[default]
    Normal,
    Improved,
}

impl ScrollbarStyle {
    pub const ALL: [ScrollbarStyle; 2] = [ScrollbarStyle::Normal, ScrollbarStyle::Improved];

    pub fn key(self) -> &'static str {
        match self {
            ScrollbarStyle::Normal => "m.scroll_normal",
            ScrollbarStyle::Improved => "m.scroll_improved",
        }
    }

    /// Applique le réglage aux deux thèmes (clair et sombre).
    pub fn apply(self, ctx: &egui::Context) {
        let min_len = match self {
            ScrollbarStyle::Normal => 12.0,
            ScrollbarStyle::Improved => 56.0,
        };
        ctx.all_styles_mut(|s| s.spacing.scroll.handle_min_length = min_len);
    }
}

/// Taille des miniatures du mode dossier (plus grand côté, en points).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum ThumbSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ThumbSize {
    pub const ALL: [ThumbSize; 3] = [ThumbSize::Small, ThumbSize::Medium, ThumbSize::Large];

    pub fn px(self) -> u32 {
        match self {
            ThumbSize::Small => 64,
            ThumbSize::Medium => 96,
            ThumbSize::Large => 128,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ThumbSize::Small => "m.thumb_small",
            ThumbSize::Medium => "m.thumb_medium",
            ThumbSize::Large => "m.thumb_large",
        }
    }
}

/// Repère d'ancrage (origine) : désactivé, point, croix ou cible (cercle et croix).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum OriginStyle {
    #[default]
    Off,
    Dot,
    Cross,
    Target,
}

impl OriginStyle {
    pub const ALL: [OriginStyle; 4] = [OriginStyle::Off, OriginStyle::Dot, OriginStyle::Cross, OriginStyle::Target];

    pub fn key(self) -> &'static str {
        match self {
            OriginStyle::Off => "m.origin_off",
            OriginStyle::Dot => "m.origin_dot",
            OriginStyle::Cross => "m.origin_cross",
            OriginStyle::Target => "m.origin_target",
        }
    }

    /// Style suivant (raccourci A) : revient à « désactivé » après la cible.
    pub fn next(self) -> OriginStyle {
        let i = OriginStyle::ALL.iter().position(|s| *s == self).unwrap_or(0);
        OriginStyle::ALL[(i + 1) % OriginStyle::ALL.len()]
    }
}

/// Thème de l'interface : celui du système, sombre ou clair.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "m.theme_system",
            ThemeChoice::Dark => "m.theme_dark",
            ThemeChoice::Light => "m.theme_light",
        }
    }
}

/// Échelle verticale de l'histogramme.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum HistoScale {
    #[default]
    Linear,
    Sqrt,
    Log,
}

impl HistoScale {
    pub fn key(self) -> &'static str {
        match self {
            HistoScale::Linear => "histo.lin",
            HistoScale::Sqrt => "histo.sqrt",
            HistoScale::Log => "histo.log",
        }
    }
}

/// Présentation de l'histogramme : barres remplies ou courbes.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum HistoStyle {
    #[default]
    Bars,
    Curve,
}

impl HistoStyle {
    pub fn key(self) -> &'static str {
        match self {
            HistoStyle::Bars => "histo.bars",
            HistoStyle::Curve => "histo.curve",
        }
    }
}

/// Réglages de l'histogramme : canaux affichés (R, V, B, luminance), échelle, remplissage, grille et indicateurs d'écrêtage.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoSettings {
    pub channels: [bool; 4],
    pub scale: HistoScale,
    pub style: HistoStyle,
    pub grid: bool,
    pub clip: bool,
}

impl Default for HistoSettings {
    fn default() -> Self {
        HistoSettings { channels: [true, true, true, false], scale: HistoScale::Sqrt, style: HistoStyle::Curve, grid: true, clip: true }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub scrollbars: ScrollbarStyle,
    /// Panneau de gauche : `false` = arborescence, `true` = mode dossier (miniatures).
    pub folder_view: bool,
    pub thumb_size: ThumbSize,
    pub theme: ThemeChoice,
    pub histo: HistoSettings,
    pub lang: Option<Lang>,
    pub show_tree: bool,
    pub show_props: bool,
    /// Bloc « Couleurs » (couleurs 1 et 2) au-dessus des propriétés.
    pub show_colors: bool,
    /// Couleurs 1 (clic gauche) et 2 (clic droit) prélevées sur l'image ou la palette (RGBA).
    pub color1: [u8; 4],
    pub color2: [u8; 4],
    /// Tri des fichiers d'un dossier.
    pub sort: crate::tree::SortKey,
    pub sort_desc: bool,
    /// Fond derrière l'image, grille de pixels, infos du pixel sous la souris, repère d'ancrage, pelure d'oignon.
    pub background: crate::view::Background,
    /// Couleur du fond « personnalisé ».
    pub bg_custom: [u8; 3],
    /// Opacité (0 à 255) du fond « personnalisé » ; en dessous de 255, le damier se voit à travers.
    pub bg_alpha: u8,
    /// Couleurs récemment choisies pour le fond personnalisé (la plus récente en premier).
    pub bg_recent: Vec<[u8; 3]>,
    pub show_grid: bool,
    pub show_pixel_info: bool,
    pub origin: OriginStyle,
    /// Origine fixe : l'ancrage reste au même endroit de l'écran quand on change d'image.
    pub lock_origin: bool,
    /// Toutes les directions d'un FRM (ou ART) côte à côte.
    pub show_all_dirs: bool,
    pub onion: bool,
    /// Délai du diaporama en secondes.
    pub slideshow_secs: u32,
    pub last_dir: Option<PathBuf>,
    /// 0 = auto, 1 = Fallout, 2 = personnalisée
    pub pal_mode: u8,
    pub pal_path: Option<PathBuf>,
    pub smoothing: Smoothing,
    /// Lecture automatique des animations (FRM, GIF...).
    pub autoplay: bool,
    /// Animation de la palette Fallout (eau, feux, écrans) pour les images qui utilisent ces couleurs.
    pub cycling: bool,
    /// Molette : false = zoom, true = image précédente / suivante.
    pub wheel_nav: bool,
    /// Boutons latéraux de la souris : false = précédente / suivante, true = zoom.
    pub xbtn_zoom: bool,
    pub conv: crate::convert_ui::ConvSettings,
    /// Conversion par lot (fenêtre indépendante).
    pub batch: crate::batch_ui::BatchSettings,
    /// Extraction de palette : dernier format (id) et dernier dossier.
    pub pal_fmt: String,
    pub pal_dir: Option<PathBuf>,
    /// Dossiers favoris (5 au plus), affichés au-dessus du filtre de l'arborescence.
    pub favorites: Vec<PathBuf>,
    /// Fenêtres supplémentaires indépendantes (lot, À propos, aide) : vraies fenêtres du système, visibles dans la barre des tâches.
    pub taskbar_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lang: None,
            show_tree: true,
            show_props: true,
            show_colors: false,
            theme: Default::default(),
            folder_view: false,
            scrollbars: ScrollbarStyle::Normal,
            thumb_size: ThumbSize::Medium,
            histo: Default::default(),
            color1: [0, 0, 0, 255],
            color2: [255, 255, 255, 255],
            sort: Default::default(),
            sort_desc: false,
            background: Default::default(),
            bg_custom: [64, 64, 64],
            bg_alpha: 255,
            bg_recent: Vec::new(),
            show_grid: false,
            show_pixel_info: true,
            origin: OriginStyle::Off,
            lock_origin: false,
            show_all_dirs: false,
            onion: false,
            slideshow_secs: 3,
            last_dir: None,
            pal_mode: 0,
            pal_path: None,
            smoothing: Smoothing::Auto,
            autoplay: false,
            cycling: false,
            wheel_nav: false,
            xbtn_zoom: false,
            conv: Default::default(),
            batch: Default::default(),
            pal_fmt: "pal".to_string(),
            pal_dir: None,
            favorites: Vec::new(),
            taskbar_windows: false,
        }
    }
}


impl crate::app::App {
    /// Efface les préférences enregistrées (registre / dossier de configuration) et revient aux réglages par défaut.
    pub fn reset_preferences(&mut self, ctx: &egui::Context) {
        crate::store::Store::open().clear();
        self.settings = Settings::default();
        self.win = crate::winprefs::WinPrefs::default();
        i18n::set_lang(Lang::detect());
        ctx.set_theme(self.settings.theme.preference());
        self.settings.scrollbars.apply(ctx);
        self.conv = crate::convert_ui::ConvertUi::new(&self.settings.conv);
        self.batch = crate::batch_ui::BatchUi::new(&self.settings.batch);
        self.pal_dlg = crate::palette_ui::PalDlg::new(&self.settings.pal_fmt);
        self.set_policy(ctx, crate::loader::PalPolicy::Auto);
        self.apply_view_options();
        self.thumbs.clear();
        if let Some(s) = &mut self.shown {
            s.tex.clear();
        }
        self.toast(tr("m.reset_done"));
    }
}
