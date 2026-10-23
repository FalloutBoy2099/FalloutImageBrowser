//! Application egui : état, navigation, chargement.

use crate::convert_ui::ConvertUi;
use crate::i18n::{self, Lang, tr};
use crate::loader::{Done, Loaded, Loader, PalPolicy, load_pal_file};
use crate::model::*;
use crate::tree::Tree;
use eframe::egui::{self, TextureHandle, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SETTINGS_KEY: &str = "fib_settings_v1";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum Smoothing {
    #[default]
    Auto,
    On,
    Off,
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
            show_props: false,
            show_colors: false,
            theme: Default::default(),
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
            show_all_dirs: false,
            onion: false,
            slideshow_secs: 3,
            last_dir: None,
            pal_mode: 0,
            pal_path: None,
            smoothing: Smoothing::Auto,
            autoplay: false,
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

/// État de la fenêtre « Renommer » : fichier, nom en cours de saisie, première image (pour sélectionner le nom).
pub struct RenameDlg {
    pub path: PathBuf,
    pub name: String,
    pub fresh: bool,
}

/// Fenêtre « Créer une animation » : dossier des images, format de sortie, nom (sans extension) et dossier de sortie.
pub struct AnimDlg {
    pub src: PathBuf,
    pub format: Format,
    pub name: String,
    pub out_dir: String,
    pub fresh: bool,
}

/// Taille maximale d'un fichier (sur le disque) préchargé avec les images voisines.
/// Favoris ajustables (les trois autres boutons sont réservés : Bureau, Documents, Téléchargements).
pub const MAX_FAVORITES: usize = 5;

const PREFETCH_MAX_BYTES: u64 = 24 << 20;

pub struct TexEntry {
    pub handle: TextureHandle,
    /// Taille en mémoire graphique (RGBA 8 bits).
    pub bytes: usize,
    /// Dernier passage d'affichage qui l'a demandée (éviction de la moins récente).
    pub used: u64,
}

pub struct Shown {
    pub loaded: Arc<Loaded>,
    pub dir: usize,
    pub frame: usize,
    /// Textures : (direction, image, niveau de réduction, transformation de la vue).
    pub tex: HashMap<(usize, usize, u8, u8), TexEntry>,
    pub playing: bool,
    pub clock: f32,
}

#[derive(Default)]
pub struct Cache {
    map: HashMap<PathBuf, Arc<Loaded>>,
    order: VecDeque<PathBuf>,
    bytes: usize,
}

fn est_bytes(l: &Loaded) -> usize {
    l.pic
        .frames
        .iter()
        .map(|f| match &f.pixels {
            Pixels::Indexed(v) => v.len(),
            Pixels::Rgba(v) => v.len(),
        })
        .sum()
}

impl Cache {
    pub fn get(&mut self, p: &Path) -> Option<Arc<Loaded>> {
        let v = self.map.get(p).cloned()?;
        self.order.retain(|x| x != p);
        self.order.push_back(p.to_path_buf());
        Some(v)
    }
    pub fn insert(&mut self, l: Arc<Loaded>) {
        let p = l.path.clone();
        if let Some(old) = self.map.insert(p.clone(), l.clone()) {
            self.bytes = self.bytes.saturating_sub(est_bytes(&old));
        }
        self.bytes += est_bytes(&l);
        self.order.retain(|x| *x != p);
        self.order.push_back(p);
        while (self.order.len() > 12 || self.bytes > 700_000_000) && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front()
                && let Some(v) = self.map.remove(&old)
            {
                self.bytes = self.bytes.saturating_sub(est_bytes(&v));
            }
        }
    }
    pub fn contains(&self, p: &Path) -> bool {
        self.map.contains_key(p)
    }
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }
}

pub struct App {
    pub settings: Settings,
    pub tree: Tree,
    pub files: Vec<PathBuf>,
    pub index: Option<usize>,
    pub wanted: Option<PathBuf>,
    pub shown: Option<Shown>,
    pub error: Option<(PathBuf, String)>,
    pub cache: Cache,
    pub loader: Loader,
    pub policy: PalPolicy,
    pub zoom: f32,
    pub pan: Vec2,
    pub fit: bool,
    /// L'ajustement peut agrandir l'image au-delà de 100 % (commande explicite « Ajuster à la fenêtre »).
    pub fit_up: bool,
    pub checker: Option<TextureHandle>,
    pub conv: ConvertUi,
    pub batch: crate::batch_ui::BatchUi,
    pub pal_dlg: crate::palette_ui::PalDlg,
    pub toast: Option<(String, Instant)>,
    pub show_about: bool,
    pub show_cli_help: bool,
    pub expand_confirm: Option<PathBuf>,
    /// Boîte « Dossier courant » : (chemin en cours d'édition, première image).
    pub folder_dlg: Option<(String, bool)>,
    /// Fenêtre « Renommer » du fichier affiché.
    pub rename_dlg: Option<RenameDlg>,
    /// Fenêtre « Créer une animation » d'un dossier.
    pub anim_dlg: Option<AnimDlg>,
    pub frm_dlg: Option<crate::frm_ui::FrmDlg>,
    /// Texte du filtre de fichiers (zone de saisie au-dessus de l'arborescence).
    pub file_filter: String,
    /// Fenêtre du sélecteur de couleur du fond personnalisé.
    pub bg_picker_open: bool,
    /// Sélecteur de couleur : fond et couleur au moment de l'ouverture (rétablis par Annuler), teinte / saturation / valeur et texte hexadécimal en cours.
    pub bg_picker_prev: (crate::view::Background, [u8; 3], u8),
    pub bg_hsv: (f32, f32, f32),
    pub bg_hex: String,
    /// Largeur du contenu de l'arborescence (version des lignes, largeur) : sert à la barre de défilement horizontale.
    pub tree_w: (u64, f32),
    /// Rotation et symétries de la vue (l'image n'est pas modifiée) ; plein écran ; diaporama et heure du dernier changement.
    pub transform: crate::view::Transform,
    pub fullscreen: bool,
    pub slideshow: bool,
    pub slideshow_t: f64,
    /// Extraction d'archive en cours (thread) : résultat et dossier de sortie.
    pub extract_job: Option<(std::sync::mpsc::Receiver<Result<crate::dat::Extracted, String>>, std::path::PathBuf)>,
    pub tree_scroll_to: Option<usize>,
    /// Bureau, Documents, Téléchargements (boutons réservés de la barre de favoris).
    pub special: Vec<(&'static str, PathBuf)>,
    /// Zone de l'arborescence (la saisie au clavier n'y agit que si la souris la survole) et recherche en cours.
    pub tree_rect: Option<egui::Rect>,
    pub ta_buf: String,
    pub ta_time: Instant,
    /// Dernier SHA-256 copié (nom du fichier, empreinte) : sert à comparer deux fichiers.
    pub ref_hash: Option<(String, String)>,
    pub last_title: String,
    pub toolbar_w: f32,
    pub win: crate::winprefs::WinPrefs,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> App {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        let forced = std::env::var("FIB_LANG").ok().and_then(|c| Lang::ALL.iter().copied().find(|l| l.code() == c.to_lowercase()));
        let lang = forced.or(settings.lang).unwrap_or_else(Lang::detect);
        i18n::set_lang(lang);
        cc.egui_ctx.set_theme(settings.theme.preference());
        let mut app = App {
            tree: Tree::new(),
            files: Vec::new(),
            index: None,
            wanted: None,
            shown: None,
            error: None,
            cache: Cache::default(),
            loader: Loader::new(cc.egui_ctx.clone()),
            policy: PalPolicy::Auto,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: true,
            fit_up: false,
            checker: None,
            conv: ConvertUi::new(&settings.conv),
            batch: crate::batch_ui::BatchUi::new(&settings.batch),
            pal_dlg: crate::palette_ui::PalDlg::new(&settings.pal_fmt),
            toast: None,
            show_about: false,
            show_cli_help: false,
            expand_confirm: None,
            folder_dlg: None,
            rename_dlg: None,
            anim_dlg: None,
            frm_dlg: None,
            file_filter: String::new(),
            bg_picker_open: false,
            bg_picker_prev: (crate::view::Background::Checker, [64, 64, 64], 255),
            bg_hsv: (0.0, 0.0, 0.25),
            bg_hex: String::new(),
            tree_w: (u64::MAX, 0.0),
            transform: Default::default(),
            fullscreen: false,
            slideshow: false,
            slideshow_t: 0.0,
            extract_job: None,
            tree_scroll_to: None,
            special: crate::tree::special_folders().into_iter().filter(|(_, p)| p.is_dir()).collect(),
            tree_rect: None,
            ta_buf: String::new(),
            ta_time: Instant::now(),
            ref_hash: None,
            last_title: String::new(),
            toolbar_w: 0.0,
            win: crate::winprefs::WinPrefs::load(),
            settings,
        };
        app.apply_fonts(&cc.egui_ctx);
        app.tree.sort = app.settings.sort;
        app.tree.descending = app.settings.sort_desc;
        match (app.settings.pal_mode, app.settings.pal_path.clone()) {
            (1, _) => app.policy = PalPolicy::Fallout,
            (2, Some(p)) => {
                if let Ok(pal) = load_pal_file(&p) {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    app.policy = PalPolicy::Custom(Arc::new(pal), name);
                }
            }
            _ => {}
        }
        let start = initial.or_else(|| app.settings.last_dir.clone());
        if let Some(p) = start {
            app.open_path(&cc.egui_ctx, &p);
        }
        app
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ------------------------------------------------------------------ polices

    /// Polices : celle d'egui + une police CJK du système en secours (pour « 中文 », les noms de fichiers...).
    pub fn apply_fonts(&mut self, ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        let candidates = [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyh.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ];
        for c in candidates {
            if let Ok(bytes) = std::fs::read(c) {
                fonts.font_data.insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
                for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(fam).or_default().push("cjk".to_owned());
                }
                break;
            }
        }
        ctx.set_fonts(fonts);
    }

    pub fn set_language(&mut self, _ctx: &egui::Context, l: Lang) {
        self.settings.lang = Some(l);
        i18n::set_lang(l);
    }

    // ------------------------------------------------------------------ navigation

    /// Ouverture depuis l'extérieur (ligne de commande, glisser-déposer, boîte de dialogue) :
    /// un fichier ouvre son dossier, qui devient le dossier courant ; une archive .DAT s'ouvre comme un dossier.
    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        let kind = match crate::dat::stat(path) {
            Some(crate::dat::Stat::File) => Some(false),
            Some(crate::dat::Stat::Dir) => Some(true),
            None if path.is_dir() => Some(true),
            None if path.is_file() => Some(false),
            None => None,
        };
        match kind {
            Some(true) => self.open_dir(ctx, path, true),
            Some(false) => {
                if let Some(parent) = path.parent() {
                    self.open_dir(ctx, parent, false);
                }
                if crate::model::is_supported(path) {
                    self.show_file(ctx, path);
                }
            }
            None => return,
        }
        self.reveal_in_tree();
    }

    pub fn open_dir(&mut self, ctx: &egui::Context, dir: &Path, open_first: bool) {
        self.tree.reveal_dir(dir);
        self.settings.last_dir = Some(dir.to_path_buf());
        self.files = self.tree.files_of(dir);
        self.index = None;
        if open_first {
            if let Some(first) = self.files.first().cloned() {
                self.show_file(ctx, &first);
            } else {
                self.wanted = None;
                self.shown = None;
                self.error = None;
            }
        } else if let Some(w) = &self.wanted {
            self.index = self.files.iter().position(|f| f == w);
        }
    }

    /// Applique le tri et le filtre choisis : la liste des fichiers (et donc précédent / suivant) suit l'arborescence.
    pub fn apply_view_options(&mut self) {
        self.tree.sort = self.settings.sort;
        self.tree.descending = self.settings.sort_desc;
        self.tree.filter = self.file_filter.clone();
        self.tree.rebuild();
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.files = self.tree.files_of(&dir);
            self.index = self.wanted.as_ref().and_then(|w| self.files.iter().position(|f| f == w));
        }
    }

    pub fn reload_dir(&mut self, ctx: &egui::Context) {
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.tree.reload_dir(&dir);
            self.files = self.tree.files_of(&dir);
            if let Some(w) = self.wanted.clone() {
                self.index = self.files.iter().position(|f| *f == w);
            }
            self.cache.clear();
            if let Some(w) = self.wanted.clone() {
                self.show_file(ctx, &w);
            }
        }
    }

    /// Recharge un dossier de l'arborescence depuis le disque (le dossier courant : liste des fichiers et image comprises).
    pub fn refresh_dir(&mut self, ctx: &egui::Context, dir: &Path) {
        if self.tree.selected_dir.as_deref() == Some(dir) {
            self.reload_dir(ctx);
        } else {
            self.tree.reload_dir(dir);
        }
    }

    /// Crée une animation FRM / ART à partir des images du dossier et l'affiche.
    pub fn create_animation(&mut self, ctx: &egui::Context, dir: &Path, out_dir: &Path, name: &str, format: Format) {
        match crate::batch::build_animation(dir, out_dir, name, format, &self.policy) {
            Ok((out, n)) => {
                self.tree.reload_dir(out_dir);
                self.open_dir(ctx, out_dir, false);
                self.show_file(ctx, &out);
                self.reveal_in_tree();
                let file = out.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                self.toast(i18n::trf("t.anim_created", &[("file", file), ("n", n.to_string())]));
            }
            Err(e) => self.toast(e),
        }
    }

    /// Fait défiler l'arborescence jusqu'à l'élément courant (une seule fois, sur demande explicite).
    pub fn reveal_in_tree(&mut self) {
        let target = self.wanted.clone().or_else(|| self.tree.selected_dir.clone());
        if let Some(t) = target {
            self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == t);
        }
    }

    pub fn show_file(&mut self, ctx: &egui::Context, path: &Path) {
        self.wanted = Some(path.to_path_buf());
        self.index = self.files.iter().position(|f| f == path);
        self.error = None;
        if let Some(l) = self.cache.get(path) {
            self.set_shown(ctx, l);
        } else {
            self.loader.request(path.to_path_buf(), self.policy.clone());
        }
    }

    pub fn step(&mut self, ctx: &egui::Context, delta: i32) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as i32;
        let cur = self.index.map(|i| i as i32).unwrap_or(if delta > 0 { -1 } else { 0 });
        let next = (cur + delta).rem_euclid(n) as usize;
        let p = self.files[next].clone();
        self.show_file(ctx, &p);
    }

    pub fn goto(&mut self, ctx: &egui::Context, idx: usize) {
        if let Some(p) = self.files.get(idx).cloned() {
            self.show_file(ctx, &p);
        }
    }

    pub fn set_shown(&mut self, ctx: &egui::Context, l: Arc<Loaded>) {
        let animated = l.pic.is_animated();
        self.shown = Some(Shown { loaded: l.clone(), dir: 0, frame: 0, tex: HashMap::new(), playing: animated && self.settings.autoplay, clock: 0.0 });
        self.pan = Vec2::ZERO;
        self.error = None;
        let name = l.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let title = format!("{name} — Fallout Image Browser");
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.prefetch_neighbours();
    }

    fn prefetch_neighbours(&mut self) {
        let Some(i) = self.index else { return };
        let n = self.files.len();
        if n < 2 {
            return;
        }
        let mut v = Vec::new();
        for d in [1i32, -1, 2, -2] {
            let j = (i as i32 + d).rem_euclid(n as i32) as usize;
            let p = self.files[j].clone();
            // Les très gros fichiers ne sont pas préchargés : leur décodage coûte beaucoup de mémoire pour une image peut-être jamais vue.
            let huge = std::fs::metadata(&p).is_ok_and(|m| m.len() > PREFETCH_MAX_BYTES);
            if !huge && !self.cache.contains(&p) && !v.contains(&p) && self.files[j] != self.files[i] {
                v.push(p);
            }
        }
        if !v.is_empty() {
            self.loader.prefetch(v, &self.policy);
        }
    }

    pub fn poll_loader(&mut self, ctx: &egui::Context) {
        while let Ok(Done { path, prefetch: _, result }) = self.loader.rx.try_recv() {
            match result {
                Ok(l) => {
                    self.cache.insert(l.clone());
                    if self.wanted.as_deref() == Some(path.as_path()) {
                        self.set_shown(ctx, l);
                    }
                }
                Err(e) => {
                    if self.wanted.as_deref() == Some(path.as_path()) {
                        self.error = Some((path, e));
                    }
                }
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        match (&self.wanted, &self.shown) {
            (Some(w), Some(s)) => s.loaded.path != *w && self.error.is_none(),
            (Some(_), None) => self.error.is_none(),
            _ => false,
        }
    }

    // ------------------------------------------------------------------ palette

    pub fn set_policy(&mut self, ctx: &egui::Context, policy: PalPolicy) {
        self.settings.pal_mode = match &policy {
            PalPolicy::Auto => 0,
            PalPolicy::Fallout => 1,
            PalPolicy::Custom(..) => 2,
        };
        self.policy = policy;
        self.cache.clear();
        if let Some(w) = self.wanted.clone() {
            self.show_file(ctx, &w);
        }
    }

    pub fn load_custom_palette(&mut self, ctx: &egui::Context) {
        let dlg = rfd::FileDialog::new().add_filter("PAL", &["pal", "PAL"]).set_title(tr("m.pal_load"));
        if let Some(p) = dlg.pick_file() {
            match load_pal_file(&p) {
                Ok(pal) => {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    self.settings.pal_path = Some(p);
                    self.set_policy(ctx, PalPolicy::Custom(Arc::new(pal), name));
                }
                Err(e) => self.toast(e),
            }
        }
    }

    // ------------------------------------------------------------------ images / animation

    pub fn dir_frame_count(&self) -> usize {
        self.shown.as_ref().map(|s| s.loaded.pic.frames_per_dir).unwrap_or(0)
    }

    pub fn set_frame(&mut self, dir: usize, frame: usize) {
        if let Some(s) = &mut self.shown {
            s.dir = dir.min(s.loaded.pic.directions.saturating_sub(1));
            s.frame = frame.min(s.loaded.pic.dir_len(s.dir).saturating_sub(1));
            s.clock = 0.0;
        }
    }

    pub fn advance_animation(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.shown else { return };
        let pic = &s.loaded.pic;
        if !(s.playing && pic.is_animated()) {
            return;
        }
        let dt = ctx.input(|i| i.unstable_dt).min(0.25);
        s.clock += dt * 1000.0;
        let n = pic.dir_len(s.dir);
        let mut guard = 0;
        loop {
            let delay = pic.frames[pic.frame_index(s.dir, s.frame)].delay_ms.unwrap_or(100).max(10) as f32;
            if s.clock < delay || guard > 64 {
                if guard > 64 {
                    s.clock = 0.0;
                }
                break;
            }
            s.clock -= delay;
            s.frame = (s.frame + 1) % n;
            guard += 1;
        }
        let delay = pic.frames[pic.frame_index(s.dir, s.frame)].delay_ms.unwrap_or(100).max(10) as f32;
        ctx.request_repaint_after(Duration::from_millis(((delay - s.clock).max(1.0)) as u64));
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.conv = self.conv.to_settings();
        self.settings.batch = self.batch.to_settings();
        self.settings.pal_fmt = self.pal_dlg.fmt.id().to_string();
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        self.win.save();
        // eframe relit toujours cette clé au démarrage : on la neutralise, la géométrie est gérée par WinPrefs.
        storage.set_string("window", String::new());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loader(&ctx);
        self.track_window(&ctx);
        self.handle_input(&ctx);
        self.advance_animation(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("toolbar").show(ui, |ui| self.toolbar(ui));
        if self.settings.show_tree {
            egui::Panel::left("tree").resizable(true).default_size(270.0).size_range(120.0..=700.0).show(ui, |ui| self.tree_panel(ui));
        }
        if self.settings.show_props || self.settings.show_colors {
            egui::Panel::right("props").resizable(true).default_size(320.0).size_range(160.0..=800.0).show(ui, |ui| {
                if self.settings.show_colors {
                    self.colors_block(ui);
                }
                if self.settings.show_props {
                    self.props_panel(ui);
                }
            });
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
        self.poll_extract(&ctx);
        self.tick_slideshow(&ctx);
        self.convert_window(&ctx);
        self.batch_window(&ctx);
        self.palette_window(&ctx);
        self.about_window(&ctx);
        self.cli_help_window(&ctx);
        self.expand_confirm_window(&ctx);
        self.folder_window(&ctx);
        self.rename_window(&ctx);
        self.anim_window(&ctx);
        self.frm_window(&ctx);
        self.bg_picker_window(&ctx);
        self.toast_overlay(&ctx);
        if self.is_loading() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

impl App {
    /// Retient la taille / position de la fenêtre (sauf agrandie, minimisée ou plein écran).
    fn track_window(&mut self, ctx: &egui::Context) {
        let (inner, outer, max, min, full) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect, v.outer_rect, v.maximized.unwrap_or(false), v.minimized.unwrap_or(false), v.fullscreen.unwrap_or(false))
        });
        if max || min || full {
            return;
        }
        if let Some(r) = inner {
            self.win.size = Some([r.width(), r.height()]);
        }
        if let Some(r) = outer {
            self.win.pos = Some([r.min.x, r.min.y]);
        }
    }
}
