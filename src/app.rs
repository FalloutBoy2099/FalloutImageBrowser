//! Application egui : état, navigation, chargement.

use crate::convert_ui::ConvertUi;
use crate::i18n::{self, Lang, tr};
use crate::loader::{Done, Loader, PalPolicy, load_pal_file};
use crate::model::*;
use crate::tree::Tree;
use eframe::egui::{self, TextureHandle, Vec2};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use crate::cache::*;
pub use crate::settings::*;

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
    pub asm_dlg: Option<crate::assemble_ui::AsmDlg>,
    pub remap_dlg: Option<crate::remap_ui::RemapDlg>,
    pub cmp_dlg: Option<crate::compare_ui::CompareDlg>,
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
    /// Miniatures du mode dossier, dernier fichier vu par ce mode et demande de défilement jusqu'à lui.
    pub thumbs: crate::thumbs::Thumbs,
    pub folder_seen: Option<PathBuf>,
    pub folder_follow: bool,
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
    /// Dernier compteur d'analyses d'archives vu, et ouverture en attente de l'analyse d'une archive (chemin ; vrai = dossier favori).
    pub dat_gen: u64,
    pub pending_open: Option<(PathBuf, PendingOpen)>,
    /// Origine fixe : position de l'ancrage à l'écran (écart au centre de la zone) et dernière vue (déplacement, zoom) qui l'a posée.
    pub origin_lock: Option<Vec2>,
    pub origin_lock_view: Option<(Vec2, f32)>,
}

/// Ce qu'il reste à faire une fois l'archive analysée.
#[derive(Clone, Copy)]
pub enum PendingOpen {
    /// `open_path` (fichier ou dossier).
    Path,
    /// `open_dir`, en affichant ou non la première image.
    Dir { open_first: bool },
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> App {
        // Réglages : magasin de préférences (registre sous Windows) ; à défaut, ancien fichier `app.ron` d'eframe.
        let mut store = crate::store::Store::open();
        let settings: Settings = match eframe::get_value(&store, SETTINGS_KEY) {
            Some(s) => s,
            None => match cc.storage.and_then(|s| eframe::get_value::<Settings>(s, SETTINGS_KEY)) {
                // Réglages d'une ancienne version (app.ron d'eframe) : migrés une fois, puis l'ancien fichier est supprimé pour
                // qu'une réinitialisation (ou la suppression de la clé) ne les fasse pas revenir.
                Some(old) => {
                    eframe::set_value(&mut store, SETTINGS_KEY, &old);
                    if let Some(dir) = eframe::storage_dir("FalloutImageBrowser") {
                        let _ = std::fs::remove_file(dir.join("app.ron"));
                    }
                    old
                }
                None => Settings::default(),
            },
        };
        let forced = std::env::var("FIB_LANG").ok().and_then(|c| Lang::ALL.iter().copied().find(|l| l.code() == c.to_lowercase()));
        let lang = forced.or(settings.lang).unwrap_or_else(Lang::detect);
        i18n::set_lang(lang);
        cc.egui_ctx.set_theme(settings.theme.preference());
        settings.scrollbars.apply(&cc.egui_ctx);
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
            asm_dlg: None,
            remap_dlg: None,
            cmp_dlg: None,
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
            thumbs: crate::thumbs::Thumbs::new(cc.egui_ctx.clone()),
            folder_seen: None,
            folder_follow: true,
            special: crate::tree::special_folders().into_iter().filter(|(_, p)| p.is_dir()).collect(),
            tree_rect: None,
            ta_buf: String::new(),
            ta_time: Instant::now(),
            ref_hash: None,
            last_title: String::new(),
            toolbar_w: 0.0,
            win: crate::winprefs::WinPrefs::load(),
            dat_gen: 0,
            pending_open: None,
            origin_lock: None,
            origin_lock_view: None,
            settings,
        };
        // Les archives volumineuses sont analysées en arrière-plan : l'interface est réveillée à la fin.
        let wake = cc.egui_ctx.clone();
        crate::dat::set_wake(move || wake.request_repaint());
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

    /// Reprend ce qui attendait l'analyse d'une archive (lancée en arrière-plan) dès qu'elle est terminée.
    pub fn poll_archives(&mut self, ctx: &egui::Context) {
        let gen_now = crate::dat::generation();
        if gen_now == self.dat_gen {
            return;
        }
        self.dat_gen = gen_now;
        if self.tree.refresh_pending() {
            self.apply_view_options();
        }
        if let Some((p, what)) = self.pending_open.take() {
            if crate::dat::archive_pending(&p) {
                self.pending_open = Some((p, what));
            } else {
                match what {
                    PendingOpen::Path => self.open_path(ctx, &p),
                    PendingOpen::Dir { open_first } => self.open_dir(ctx, &p, open_first),
                }
            }
        }
    }

    /// Ouvre un dossier favori : réel ou à l'intérieur d'une archive, dont l'analyse ne fige pas l'interface.
    pub fn open_fav_dir(&mut self, ctx: &egui::Context, p: &Path) {
        if crate::dat::archive_pending(p) || crate::dat::stat_nowait(p).is_some() || p.is_dir() {
            self.open_dir(ctx, p, false);
            // Faire défiler jusqu'au dossier choisi (et non jusqu'au fichier affiché, qui peut être ailleurs).
            self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == p);
        } else {
            self.toast(tr("t.folder_missing"));
        }
    }

    pub fn is_loading(&self) -> bool {
        if crate::dat::busy() {
            return true;
        }
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
        self.thumbs.clear();
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
    /// Rien d'autre que les préférences de `store` : l'état interne d'egui n'est pas conservé sur le disque.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.conv = self.conv.to_settings();
        self.settings.batch = self.batch.to_settings();
        self.settings.pal_fmt = self.pal_dlg.fmt.id().to_string();
        eframe::set_value(&mut crate::store::Store::open(), SETTINGS_KEY, &self.settings);
        self.win.save();
        // Les réglages ne vivent plus dans le stockage d'eframe : on y retire l'ancienne clé, que eframe réécrirait sinon.
        storage.remove_string(SETTINGS_KEY);
        // eframe relit toujours cette clé au démarrage : on la neutralise, la géométrie est gérée par WinPrefs.
        storage.set_string("window", String::new());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loader(&ctx);
        self.poll_archives(&ctx);
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
        self.assemble_window(&ctx);
        self.remap_window(&ctx);
        self.compare_window(&ctx);
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
