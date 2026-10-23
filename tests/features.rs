//! Retouches de conversion (taille, marges, tramage), conversion par lot, toutes les directions côte à côte.

use fallout_image_browser::batch::{self, BatchItem, BatchOut, Job, Progress};
use fallout_image_browser::convert::*;
use fallout_image_browser::formats::{self, DecodeCtx, EncodeOpts};
use fallout_image_browser::imageops::*;
use fallout_image_browser::loader::PalPolicy;
use fallout_image_browser::model::*;
use fallout_image_browser::palette::{self, Mapper};
use fallout_image_browser::view::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

fn ctx() -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() }
}

fn rgba_picture(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Picture {
    let mut v = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&f(x, y));
        }
    }
    Picture::single(Format::Png, w as u32, h as u32, Pixels::Rgba(v), None)
}

fn rgba_of(p: &Picture, i: usize) -> Vec<u8> {
    frame_to_rgba(&p.frames[i], p.palette.as_ref())
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fib_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn opts(format: Format, depth: DepthChoice, ops: ImageOps) -> ConvertOptions {
    ConvertOptions { format, depth, dither: false, bg: [0, 0, 0], enc: EncodeOpts::default(), ops, text_white: false }
}

fn encode(pic: &Picture, o: &ConvertOptions) -> Vec<u8> {
    convert_to_bytes(pic, o, &Selection::default()).unwrap()
}

// ----------------------------------------------------------------------------- taille et marges

#[test]
fn resize_changes_the_size_by_percentage_or_longest_side() {
    let pic = rgba_picture(100, 40, |x, y| [x as u8, y as u8, 7, 255]);
    let half = apply_ops(&pic, &ImageOps { resize: Resize::Percent(50), ..Default::default() }).unwrap();
    assert_eq!((half.frames[0].width, half.frames[0].height), (50, 20));
    let double = apply_ops(&pic, &ImageOps { resize: Resize::Percent(200), ..Default::default() }).unwrap();
    assert_eq!((double.frames[0].width, double.frames[0].height), (200, 80));
    // Plus grand côté : réduit, mais n'agrandit jamais.
    let fit = apply_ops(&pic, &ImageOps { resize: Resize::MaxSide(50), ..Default::default() }).unwrap();
    assert_eq!((fit.frames[0].width, fit.frames[0].height), (50, 20));
    let same = apply_ops(&pic, &ImageOps { resize: Resize::MaxSide(500), ..Default::default() }).unwrap();
    assert_eq!((same.frames[0].width, same.frames[0].height), (100, 40));
    // Une image qui ne change pas reste identique.
    assert_eq!(rgba_of(&same, 0), rgba_of(&pic, 0));
    // La conversion complète produit bien une image de la bonne taille.
    let bytes = encode(&pic, &opts(Format::Png, DepthChoice::Rgba32, ImageOps { resize: Resize::Percent(25), ..Default::default() }));
    let back = formats::decode(Format::Png, &bytes, &ctx()).unwrap();
    assert_eq!((back.frames[0].width, back.frames[0].height), (25, 10));
}

#[test]
fn smooth_resize_keeps_colours_of_transparent_edges_and_pixel_mode_is_exact() {
    // Moitié gauche : blanc opaque ; moitié droite : transparente (noire dans les données).
    let pic = rgba_picture(8, 8, |x, _| if x < 4 { [255, 255, 255, 255] } else { [0, 0, 0, 0] });
    let small = apply_ops(&pic, &ImageOps { resize: Resize::Percent(50), resample: Resample::Smooth, ..Default::default() }).unwrap();
    let v = rgba_of(&small, 0);
    for p in v.chunks_exact(4) {
        // Aucun gris : la couleur est soit le blanc, soit absente (alpha nul).
        assert!(p[3] == 0 || p[..3] == [255, 255, 255] || p[3] < 255, "{p:?}");
        if p[3] > 200 {
            assert!(p[..3].iter().all(|c| *c > 240), "bord assombri : {p:?}");
        }
    }
    // Plus proche voisin : chaque pixel est dupliqué tel quel.
    let pic = rgba_picture(3, 2, |x, y| [(x * 80) as u8, (y * 100) as u8, 9, 255]);
    let big = apply_ops(&pic, &ImageOps { resize: Resize::Percent(200), resample: Resample::Pixel, ..Default::default() }).unwrap();
    let (a, b) = (rgba_of(&pic, 0), rgba_of(&big, 0));
    assert_eq!((big.frames[0].width, big.frames[0].height), (6, 4));
    for y in 0..4 {
        for x in 0..6 {
            assert_eq!(b[(y * 6 + x) * 4..][..4], a[((y / 2) * 3 + x / 2) * 4..][..4], "({x},{y})");
        }
    }
}

#[test]
fn trimming_removes_transparent_margins_and_keeps_alignment() {
    // Une image 10 × 8 dont seul le bloc (3..7, 2..5) est visible, positionnée en (100, 50).
    let mut pic = rgba_picture(10, 8, |x, y| if (3..7).contains(&x) && (2..5).contains(&y) { [200, 10, 10, 255] } else { [0, 0, 0, 0] });
    pic.frames[0].pos = (100, 50);
    let t = apply_ops(&pic, &ImageOps { trim: true, ..Default::default() }).unwrap();
    assert_eq!((t.frames[0].width, t.frames[0].height), (4, 3));
    assert_eq!(t.frames[0].pos, (103, 52), "la position suit les marges retirées");
    assert!(rgba_of(&t, 0).chunks_exact(4).all(|p| p == [200, 10, 10, 255]));
    // Entièrement transparente : rien n'est retiré.
    let empty = rgba_picture(4, 4, |_, _| [0, 0, 0, 0]);
    let e = apply_ops(&empty, &ImageOps { trim: true, ..Default::default() }).unwrap();
    assert_eq!((e.frames[0].width, e.frames[0].height), (4, 4));
    // Retrait puis réduction : la position est mise à l'échelle avec le reste.
    let both = apply_ops(&pic, &ImageOps { trim: true, resize: Resize::Percent(50), resample: Resample::Pixel, ..Default::default() }).unwrap();
    assert_eq!(both.frames[0].pos, (52, 26));
    assert_eq!((both.frames[0].width, both.frames[0].height), (2, 2));
}

#[test]
fn animation_frames_keep_their_relative_position_after_resize() {
    let data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join("fallout").join("HMJMPSAA.FRM")).unwrap();
    let pic = formats::decode(Format::Frm, &data, &ctx()).unwrap();
    let r = apply_ops(&pic, &ImageOps { resize: Resize::Percent(50), resample: Resample::Pixel, ..Default::default() }).unwrap();
    assert_eq!(r.frames.len(), pic.frames.len());
    for (a, b) in pic.frames.iter().zip(&r.frames) {
        assert_eq!(b.pos, ((a.pos.0 as f64 * 0.5).round() as i32, (a.pos.1 as f64 * 0.5).round() as i32));
        assert_eq!((b.width, b.height), (((a.width as f64 * 0.5).round() as u32).max(1), ((a.height as f64 * 0.5).round() as u32).max(1)));
    }
    // Reconvertible en FRM (avec la palette Fallout) et relisible.
    let o = opts(Format::Frm, DepthChoice::Indexed(PalChoice::Fallout), ImageOps { resize: Resize::Percent(50), resample: Resample::Pixel, ..Default::default() });
    let bytes = convert_to_bytes(&pic, &o, &Selection::default()).unwrap();
    let back = formats::decode(Format::Frm, &bytes, &ctx()).unwrap();
    assert_eq!((back.directions, back.frames_per_dir), (pic.directions, pic.frames_per_dir));
}

#[test]
fn oversized_results_are_refused() {
    let pic = rgba_picture(64, 64, |_, _| [1, 2, 3, 255]);
    assert!(apply_ops(&pic, &ImageOps { resize: Resize::Percent(1000), ..Default::default() }).is_ok());
    let wide = rgba_picture(40_000, 1, |_, _| [1, 2, 3, 255]);
    assert!(apply_ops(&wide, &ImageOps { resize: Resize::Percent(100), trim: false, ..Default::default() }).is_ok(), "sans changement : copie telle quelle");
    assert!(apply_ops(&wide, &ImageOps { resize: Resize::Percent(200), ..Default::default() }).is_err());
}

#[test]
fn ops_helpers() {
    assert!(ImageOps::default().is_noop());
    assert!(!ImageOps { trim: true, ..Default::default() }.is_noop());
    assert_eq!(ImageOps { resize: Resize::Percent(25), ..Default::default() }.scale(1000), 0.25);
    assert_eq!(ImageOps { resize: Resize::MaxSide(100), ..Default::default() }.scale(400), 0.25);
    assert_eq!(ImageOps { resize: Resize::MaxSide(100), ..Default::default() }.scale(50), 1.0);
    assert_eq!(DitherMethod::from_name("Bayer"), Some(DitherMethod::Bayer));
    assert_eq!(DitherMethod::from_name("fs"), Some(DitherMethod::FloydSteinberg));
    assert_eq!(DitherMethod::from_name("x"), None);
    assert_eq!(Resample::from_name("PIXEL"), Some(Resample::Pixel));
    assert_eq!(Resample::from_name("smooth"), Some(Resample::Smooth));
    assert_eq!(Resample::from_name("?"), None);
}

// ----------------------------------------------------------------------------- tramage

#[test]
fn bayer_dithering_is_ordered_deterministic_and_keeps_transparency() {
    let pal = palette::fallout();
    let mapper = Mapper::shared(&pal, Some(0), true).unwrap();
    // Dégradé horizontal doux : sans tramage, peu de couleurs ; avec, le motif de Bayer mélange les couleurs voisines.
    let (w, h) = (64usize, 16usize);
    let mut img = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let g = 60 + (x * 2) as u8;
            img.extend_from_slice(&[g, g / 2, 40 + y as u8, if x == 0 && y == 0 { 0 } else { 255 }]);
        }
    }
    let plain = palette::quantize_rgba_with(&img, w, h, &pal, &mapper, None);
    let bayer = palette::quantize_rgba_with(&img, w, h, &pal, &mapper, Some(DitherMethod::Bayer));
    let fs = palette::quantize_rgba_with(&img, w, h, &pal, &mapper, Some(DitherMethod::FloydSteinberg));
    assert_ne!(plain, bayer);
    assert_ne!(bayer, fs);
    assert_eq!(bayer, palette::quantize_rgba_with(&img, w, h, &pal, &mapper, Some(DitherMethod::Bayer)), "déterministe");
    assert_eq!(bayer[0], 0, "le pixel transparent reste sur l'index transparent");
    let distinct = |v: &[u8]| v.iter().collect::<std::collections::HashSet<_>>().len();
    assert!(distinct(&bayer) >= distinct(&plain));
    // L'ancienne fonction booléenne reste Floyd-Steinberg.
    assert_eq!(palette::quantize_rgba(&img, w, h, &pal, &mapper, true), fs);
    assert_eq!(palette::quantize_rgba(&img, w, h, &pal, &mapper, false), plain);
}

#[test]
fn conversion_uses_the_chosen_dither_method() {
    let pic = rgba_picture(64, 16, |x, y| [(x * 4) as u8, (y * 8) as u8, 90, 255]);
    let mk = |dither: bool, method: DitherMethod| {
        let mut o = opts(Format::Bmp, DepthChoice::Indexed(PalChoice::Fallout), ImageOps { dither_method: method, ..Default::default() });
        o.dither = dither;
        encode(&pic, &o)
    };
    let (none, fs, bayer) = (mk(false, DitherMethod::Bayer), mk(true, DitherMethod::FloydSteinberg), mk(true, DitherMethod::Bayer));
    assert_ne!(none, fs);
    assert_ne!(none, bayer);
    assert_ne!(fs, bayer);
    assert_eq!(none, mk(false, DitherMethod::FloydSteinberg), "sans tramage, la méthode n'a aucun effet");
}

// ----------------------------------------------------------------------------- ligne de commande

#[test]
fn command_line_resize_and_trim_options() {
    let dir = temp_dir("cli_ops");
    let pic = rgba_picture(64, 32, |x, y| if (8..40).contains(&x) && (4..20).contains(&y) { [10, 200, 10, 255] } else { [0, 0, 0, 0] });
    let src = dir.join("in.png");
    std::fs::write(&src, encode(&pic, &opts(Format::Png, DepthChoice::Rgba32, ImageOps::default()))).unwrap();
    let args = |extra: &[&str], out: &str| -> Vec<String> {
        let mut v: Vec<String> = ["convert", src.to_str().unwrap(), "-f", "png", "-d", "32", "-q", "-o", dir.join(out).to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    let size_of = |out: &str| {
        let bytes = std::fs::read(dir.join(out).join("in.png")).unwrap();
        let p = formats::decode(Format::Png, &bytes, &ctx()).unwrap();
        (p.frames[0].width, p.frames[0].height)
    };
    assert_eq!(fallout_image_browser::cli::run(&args(&["--scale", "50"], "a")), 0);
    assert_eq!(size_of("a"), (32, 16));
    assert_eq!(fallout_image_browser::cli::run(&args(&["--max-size", "16"], "b")), 0);
    assert_eq!(size_of("b"), (16, 8));
    assert_eq!(fallout_image_browser::cli::run(&args(&["--trim"], "c")), 0);
    assert_eq!(size_of("c"), (32, 16));
    assert_eq!(fallout_image_browser::cli::run(&args(&["--trim", "--scale", "50", "--resample", "pixel", "--png-level", "fast"], "d")), 0);
    assert_eq!(size_of("d"), (16, 8));
    for bad in [&["--scale", "0"][..], &["--scale", "x"], &["--max-size", "0"], &["--resample", "blurry"], &["--dither-method", "zigzag"]] {
        assert_eq!(fallout_image_browser::cli::run(&args(bad, "z")), 2, "{bad:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------------------- lot

fn job(format: Format) -> Job {
    Job { opts: opts(format, if format == Format::Png { DepthChoice::Rgba32 } else { DepthChoice::Rgb24 }, ImageOps::default()), seq: false, policy: PalPolicy::Fallout }
}

fn write_image(path: &Path, w: usize) {
    let pic = rgba_picture(w, 4, |x, y| [x as u8, y as u8, 3, 255]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, encode(&pic, &opts(Format::Png, DepthChoice::Rgba32, ImageOps::default()))).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

#[test]
fn batch_converts_everything_counts_failures_and_never_overwrites() {
    let dir = temp_dir("batch");
    let (src, out) = (dir.join("src"), dir.join("out"));
    write_image(&src.join("a.png"), 8);
    write_image(&src.join("sub").join("a.png"), 9); // même nom dans un sous-dossier
    write_image(&src.join("sub").join("b.png"), 10);
    std::fs::write(src.join("broken.png"), b"this is not a png").unwrap();
    std::fs::write(src.join("notes.txt"), b"ignored").unwrap();

    // Un seul dossier, sans l'arborescence : les deux « a.png » ne s'écrasent pas.
    let (list, dir_mode, _) = batch::list_inputs(&src, true).unwrap();
    assert!(dir_mode);
    assert_eq!(list.len(), 4, "les images et le fichier cassé, pas le .txt : {list:?}");
    let items: Vec<BatchItem> = list.into_iter().map(|(path, rel)| BatchItem { path, rel }).collect();
    let progress = Progress::new(items.len());
    batch::run_batch(&items, &job(Format::Bmp), &BatchOut::Folder { dir: out.clone(), keep_tree: false }, &Selection::default(), &progress);
    assert!(progress.finished.load(Ordering::Acquire));
    assert_eq!(progress.done.load(Ordering::Relaxed), 4);
    assert_eq!(progress.failed.load(Ordering::Relaxed), 1);
    assert_eq!(progress.written.load(Ordering::Relaxed), 3);
    assert_eq!(progress.errors.lock().unwrap().len(), 1);
    assert!(progress.errors.lock().unwrap()[0].contains("broken.png"));
    assert_eq!(names(&out), ["a.bmp", "a_converted.bmp", "b.bmp"]);

    // Avec l'arborescence : les sous-dossiers sont recréés.
    let out2 = dir.join("out2");
    let progress = Progress::new(items.len());
    batch::run_batch(&items, &job(Format::Png), &BatchOut::Folder { dir: out2.clone(), keep_tree: true }, &Selection::default(), &progress);
    assert_eq!(names(&out2), ["a.png", "sub"]);
    assert_eq!(names(&out2.join("sub")), ["a.png", "b.png"]);

    // À côté des sources.
    let progress = Progress::new(items.len());
    batch::run_batch(&items, &job(Format::Tga), &BatchOut::NextToSource, &Selection::default(), &progress);
    assert!(src.join("a.tga").is_file() && src.join("sub").join("b.tga").is_file());
    assert_eq!(progress.failed.load(Ordering::Relaxed), 1);

    // Relancer le même lot ne remplace rien.
    let before = names(&src);
    let progress = Progress::new(items.len());
    batch::run_batch(&items, &job(Format::Tga), &BatchOut::NextToSource, &Selection::default(), &progress);
    assert!(src.join("a_converted.tga").is_file());
    assert!(names(&src).len() > before.len());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_can_be_cancelled_and_applies_the_retouches() {
    let dir = temp_dir("batch_cancel");
    let src = dir.join("src");
    for i in 0..6 {
        write_image(&src.join(format!("img{i}.png")), 16);
    }
    let (list, _, _) = batch::list_inputs(&src, false).unwrap();
    let items: Vec<BatchItem> = list.into_iter().map(|(path, rel)| BatchItem { path, rel }).collect();
    // Annulé avant de commencer : rien n'est écrit.
    let progress = Progress::new(items.len());
    progress.cancel();
    let out = dir.join("out");
    batch::run_batch(&items, &job(Format::Bmp), &BatchOut::Folder { dir: out.clone(), keep_tree: false }, &Selection::default(), &progress);
    assert_eq!(progress.done.load(Ordering::Relaxed), 0);
    assert!(progress.finished.load(Ordering::Acquire));
    assert!(!out.exists() || names(&out).is_empty());
    // Avec réduction de moitié.
    let mut j = job(Format::Png);
    j.opts.ops.resize = Resize::Percent(50);
    let progress = Progress::new(items.len());
    batch::run_batch(&items, &j, &BatchOut::Folder { dir: out.clone(), keep_tree: false }, &Selection::default(), &progress);
    assert_eq!(progress.done.load(Ordering::Relaxed), 6);
    let p = formats::decode(Format::Png, &std::fs::read(out.join("img0.png")).unwrap(), &ctx()).unwrap();
    assert_eq!((p.frames[0].width, p.frames[0].height), (8, 2));
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------------------- toutes les directions

fn frm() -> Picture {
    let data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join("fallout").join("HMJMPSAA.FRM")).unwrap();
    formats::decode(Format::Frm, &data, &ctx()).unwrap()
}

#[test]
fn all_directions_are_laid_out_in_aligned_cells() {
    let pic = frm();
    assert_eq!(pic.directions, 6);
    assert!(can_show_all_dirs(&pic));
    let g = dir_grid(&pic);
    assert_eq!((g.n, g.cols, g.rows), (6, 3, 2));
    let (w, h) = g.size();
    assert_eq!((w, h), (3 * g.cell_w, 2 * g.cell_h));
    // Toutes les images de toutes les directions tiennent dans une cellule.
    for d in 0..6 {
        let b = pic.dir_bounds(d);
        assert!(b.0 >= g.x0 && b.1 >= g.y0 && b.2 - g.x0 <= g.cell_w as i32 && b.3 - g.y0 <= g.cell_h as i32, "direction {d}");
    }
    // Position des cellules et recherche d'une cellule par pixel.
    assert_eq!(g.cell_origin(0), (0, 0));
    assert_eq!(g.cell_origin(4), (g.cell_w, g.cell_h));
    assert_eq!(g.cell_at(0, 0), Some((0, 0, 0)));
    assert_eq!(g.cell_at(g.cell_w as i32 + 3, g.cell_h as i32 + 5), Some((4, 3, 5)));
    assert_eq!(g.cell_at(-1, 0), None);
    assert_eq!(g.cell_at(w as i32, 0), None);
    assert_eq!(g.anchor(0), (-g.x0, -g.y0));
}

#[test]
fn composite_pixels_match_each_direction_for_every_frame() {
    let pic = frm();
    let g = dir_grid(&pic);
    for f in 0..pic.frames_per_dir {
        let (w, h, rgba) = render_all_dirs(&pic, f);
        assert_eq!((w, h), (g.size().0 as usize, g.size().1 as usize));
        for y in 0..h {
            for x in 0..w {
                let px: [u8; 4] = rgba[(y * w + x) * 4..][..4].try_into().unwrap();
                let (d, lx, ly) = g.cell_at(x as i32, y as i32).unwrap();
                let b = pic.dir_bounds(d);
                let info = pixel_info(&pic, d, f, lx - (b.0 - g.x0), ly - (b.1 - g.y0));
                // Hors du canevas de la direction : rien n'est dessiné.
                let expect = info.map_or([0; 4], |i| i.rgba);
                assert_eq!(px, if expect[3] == 0 { [0; 4] } else { expect }, "image {f}, ({x},{y}), direction {d}");
            }
        }
    }
}

#[test]
fn single_direction_images_cannot_show_all_directions() {
    let pic = rgba_picture(10, 10, |_, _| [1, 2, 3, 255]);
    assert!(!can_show_all_dirs(&pic));
    let mut many = pic.clone();
    many.directions = MAX_GRID_DIRS + 1;
    assert!(!can_show_all_dirs(&many));
    many.directions = 2;
    assert!(can_show_all_dirs(&many));
    // Trois directions ou moins : une seule rangée.
    let g = dir_grid(&many);
    assert_eq!((g.cols, g.rows), (2, 1));
}

#[test]
fn estimate_sizes_are_written_briefly() {
    use fallout_image_browser::convert_ui::short_size;
    assert_eq!(short_size(512), "512 B");
    assert_eq!(short_size(2048), "2.0 KiB");
    assert_eq!(short_size(3 * 1024 * 1024), "3.00 MiB");
}

// ----------------------------------------------------------------------------- palette créée : index 220 blanc

#[test]
fn created_palette_can_force_index_220_to_white() {
    // 200 couleurs sombres distinctes : la palette exacte occupe les index 1..=200 ; aucune n'est blanche.
    let pic = rgba_picture(20, 10, |x, y| [(x * 8) as u8, (y * 20) as u8, 40, 255]);
    let mut o = opts(Format::Frm, DepthChoice::Indexed(PalChoice::Create), ImageOps::default());
    let free = prepare(&pic, &o, &Selection::default()).unwrap().palette.unwrap();
    o.text_white = true;
    let forced = prepare(&pic, &o, &Selection::default()).unwrap().palette.unwrap();
    assert_ne!(forced.colors[220], free.colors[220]);
    assert_eq!(&forced.colors[220][..3], &[252, 252, 252]);
    assert!(forced.used[220]);
}
