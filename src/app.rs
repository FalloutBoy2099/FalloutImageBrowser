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

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub lang: Option<Lang>,
    pub show_tree: bool,
    pub show_props: bool,
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
    /// Extraction de palette : dernier format (id) et dernier dossier.
    pub pal_fmt: String,
    pub pal_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lang: None,
            show_tree: true,
            show_props: false,
            last_dir: None,
            pal_mode: 0,
            pal_path: None,
            smoothing: Smoothing::Auto,
            autoplay: false,
            wheel_nav: false,
            xbtn_zoom: false,
            conv: Default::default(),
            pal_fmt: "pal".to_string(),
            pal_dir: None,
        }
    }
}

pub struct TexEntry {
    pub handle: TextureHandle,
}

pub struct Shown {
    pub loaded: Arc<Loaded>,
    pub dir: usize,
    pub frame: usize,
    pub tex: HashMap<(usize, usize, u8), TexEntry>,
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
            if let Some(old) = self.order.pop_front() {
                if let Some(v) = self.map.remove(&old) {
                    self.bytes = self.bytes.saturating_sub(est_bytes(&v));
                }
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
    pub checker: Option<TextureHandle>,
    pub conv: ConvertUi,
    pub pal_dlg: crate::palette_ui::PalDlg,
    pub toast: Option<(String, Instant)>,
    pub show_about: bool,
    pub show_cli_help: bool,
    pub expand_confirm: Option<PathBuf>,
    /// Boîte « Dossier courant » : (chemin en cours d'édition, première image).
    pub folder_dlg: Option<(String, bool)>,
    pub tree_scroll_to: Option<usize>,
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
            checker: None,
            conv: ConvertUi::new(&settings.conv),
            pal_dlg: crate::palette_ui::PalDlg::new(&settings.pal_fmt),
            toast: None,
            show_about: false,
            show_cli_help: false,
            expand_confirm: None,
            folder_dlg: None,
            tree_scroll_to: None,
            last_title: String::new(),
            toolbar_w: 0.0,
            win: crate::winprefs::WinPrefs::load(),
            settings,
        };
        app.apply_fonts(&cc.egui_ctx);
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
            if !self.cache.contains(&p) && !v.contains(&p) && self.files[j] != self.files[i] {
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
        if self.settings.show_props {
            egui::Panel::right("props").resizable(true).default_size(320.0).size_range(160.0..=800.0).show(ui, |ui| self.props_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
        self.convert_window(&ctx);
        self.palette_window(&ctx);
        self.about_window(&ctx);
        self.cli_help_window(&ctx);
        self.expand_confirm_window(&ctx);
        self.folder_window(&ctx);
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
