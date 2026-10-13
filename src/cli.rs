//! Mode ligne de commande : `fallout_image_browser convert <fichier|dossier|archive.dat> --format <fmt> …`

use crate::batch::{self, Job};
use crate::convert::*;
use crate::formats::{self, EncodeOpts, PngLevel};
use crate::imageops::{DitherMethod, ImageOps, Resample, Resize};
use crate::loader::{PalPolicy, load_pal_file};
use crate::model::*;
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const USAGE: &str = "\
Fallout Image Browser - viewer and converter
Read and write: ART, BMP, DDS, FRM, GIF, JPEG, PCX, PNG, PNM, QOI, RIX, TIFF, TGA, WEBP
Read only (usable as INPUT, never as --format): PSD, PDN, XCF, KRA, ORA (merged image without layers), ASE (Aseprite),
  Fallout Tactics ZAR, TIL and SPR, palette files PAL, ACT, GPL, KPL, HEX (shown as a colour grid)
Animated WebP, PNG (APNG) and GIF keep their frames (see --frame, --sequence).

USAGE
  fallout_image_browser [PATH]
      Open the viewer on a file, a folder or a .DAT archive.

  fallout_image_browser convert <INPUT> --format <FMT> [OPTIONS]
      File mode:       INPUT is one image file (it can be inside a .DAT: archive.dat\\art\\x.frm).
      Directory mode:  INPUT is one folder, one .DAT archive or one folder inside an archive;
                       every supported image directly inside it is converted (add -r for sub-folders).

  fallout_image_browser palette <INPUT> --format <PF> [-o FILE_OR_DIR] [--read-pal P]
      Extract the palette of one image to a palette file. PF: pal (Fallout) | riff (Microsoft RIFF .pal)
      | act (Adobe) | gpl (GIMP) | kpl (Krita) | hex. INPUT may itself be a palette file (PAL, ACT, GPL, KPL,
      HEX): this converts one palette format to another.

  fallout_image_browser extract <INPUT> [-o DIR] [--overwrite] [-q]
      Extract files from a .DAT archive without converting them. INPUT is an archive, a folder inside it
      (archive.dat\\art\\intrface) or one file; the sub-folder layout is kept. Default output folder:
      ./<archive name>. Existing files are kept unless --overwrite is given.

  fallout_image_browser --formats       list the formats and the colour depths each one accepts
  fallout_image_browser /?              this help        (also: -h, --help, -?, help)
  fallout_image_browser --version

OPTIONS (convert)
  -f, --format <FMT>     art | bmp | dds | frm | gif | jpg | pcx | png | pnm | qoi | rix | tga | tif | webp    (required)
  -d, --depth <D>        8   own palette (kept as is when the image is already 8-bit)
                         8f  Fallout palette (default.pal)
                         8p  (frm only) palette file: --pal if given, otherwise a palette is created from the
                             image and written next to each result as NAME.pal
                         16 | 24 | 32   16-bit (RGB 5-5-5), 24-bit, 32-bit (with alpha; CMYK for JPEG)
                         Default: 8f for frm, 8 for art/rix/gif, 24 for the others (32 for png/qoi/webp with alpha).
  -p, --pal <FILE.PAL>   custom palette: used to read FRM files and for --depth 8p
      --read-pal <P>     palette used to READ indexed FRM files: auto (same-name .PAL, else Fallout),
                         fallout, or a .PAL file (default: --pal if given, else auto)
  -o, --out <DIR>        output folder (default: next to the source; for an archive: ./<archive name>)
  -n, --name <NAME>      output file name without extension (file mode only)
  -r, --recursive        also convert sub-folders; the sub-folder layout is kept under --out
                         (always the case for a whole .DAT archive)
      --dither           dithering for 8-bit output (Floyd-Steinberg unless --dither-method says otherwise)
      --dither-method <M>  fs (error diffusion, default) or bayer (ordered); implies --dither
      --scale <PERCENT>  resize to this percentage of the original size (1-1000)
      --max-size <PX>    shrink so that the longest side is at most PX (never enlarges)
      --resample <M>     smooth (default, Lanczos) or pixel (nearest neighbour, keeps sharp pixels)
      --trim             remove the fully transparent margins of each image
      --quality <1-100>  JPEG quality (default 92)
      --png-level <L>    PNG compression: fast, normal (default) or best; fast is much quicker on large photos
      --bg <RRGGBB>      colour replacing transparency when the format has none (default 000000)
      --direction <N>    direction to export (1-based) for single-image formats (default 1)
      --frame <N>        frame to export (1-based) for single-image formats (default 1)
      --sequence         write EVERY frame of an animation as a numbered image (single-image formats)
  -q, --quiet            only print errors and the final summary
      --lang <xx>        language of the messages (default: system language)

Existing files are never overwritten: a suffix _converted, _converted_2, ... is added.
Exit status: 0 = everything converted, 1 = at least one failure, 2 = usage error.

EXAMPLES
  fallout_image_browser convert critter.frm -f gif -d 8f -o out
  fallout_image_browser convert photo.jpg -f frm -d 8p                  (creates photo.frm + photo.pal)
  fallout_image_browser convert D:\\Fallout2\\master.dat -f bmp -d 24 -o export
  fallout_image_browser convert D:\\art\\scenery -f frm -d 8p -p my.pal -r
  fallout_image_browser convert gui\\charcreate.zar -f png -d 32         (Fallout Tactics ZAR to PNG)
  fallout_image_browser convert sprites\\Mace.spr -f png --sequence      (every frame of a Fallout Tactics SPR)
  fallout_image_browser palette default.pal -f gpl                      (palette file to GIMP palette)
  fallout_image_browser extract D:\\Fallout2\\master.dat -o master        (every file of the archive)
  fallout_image_browser extract master.dat\\art\\intrface -o ui           (one folder of the archive)
";

/// Nom de l'exécutable utilisé (sans extension) : l'aide s'adapte s'il est renommé.
pub fn exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "fallout_image_browser".to_string())
}

/// Texte d'aide avec le nom réel de l'exécutable.
pub fn usage() -> String {
    USAGE.replace("fallout_image_browser", &exe_name())
}

pub fn is_cli(args: &[String]) -> bool {
    matches!(
        args.first().map(|s| s.as_str()),
        Some("convert")
            | Some("palette")
            | Some("extract")
            | Some("-h")
            | Some("--help")
            | Some("help")
            | Some("/?")
            | Some("-?")
            | Some("--formats")
            | Some("--version")
            | Some("-V")
    )
}

fn parse_format(s: &str) -> Option<Format> {
    Format::from_ext(s.trim_start_matches('.')).filter(|f| f.is_writable())
}

fn default_depth(f: Format) -> DepthChoice {
    let c = formats::caps(f);
    match f {
        Format::Frm => DepthChoice::Indexed(PalChoice::Fallout),
        _ if c.indexed && !c.d24 && !c.d32 => DepthChoice::Indexed(if c.no_adaptive { PalChoice::Fallout } else { PalChoice::Adaptive }),
        _ if c.d24 => DepthChoice::Rgb24,
        _ => DepthChoice::Rgba32,
    }
}

struct Args {
    input: PathBuf,
    format: Format,
    depth: Option<String>,
    pal: Option<PathBuf>,
    read_pal: Option<String>,
    out: Option<PathBuf>,
    name: Option<String>,
    recursive: bool,
    dither: bool,
    dither_method: DitherMethod,
    resize: Resize,
    resample: Resample,
    trim: bool,
    quality: u8,
    png_level: PngLevel,
    bg: [u8; 3],
    seq: bool,
    dir: usize,
    frame: usize,
    quiet: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut it = args.iter().skip(1); // saute « convert »
    let mut a = Args {
        input: PathBuf::new(),
        format: Format::Bmp,
        depth: None,
        pal: None,
        read_pal: None,
        out: None,
        name: None,
        recursive: false,
        dither: false,
        dither_method: DitherMethod::FloydSteinberg,
        resize: Resize::None,
        resample: Resample::Smooth,
        trim: false,
        quality: 92,
        png_level: PngLevel::Normal,
        bg: [0, 0, 0],
        seq: false,
        dir: 0,
        frame: 0,
        quiet: false,
    };
    let (mut have_input, mut have_format) = (false, false);
    while let Some(arg) = it.next() {
        let mut val = |what: &str| it.next().cloned().ok_or_else(|| format!("missing value for {what}"));
        let index =
            |s: String, what: &str| -> Result<usize, String> { s.parse::<usize>().ok().filter(|n| *n >= 1).map(|n| n - 1).ok_or_else(|| format!("{what} expects a number >= 1")) };
        match arg.as_str() {
            "-f" | "--format" => {
                let v = val("--format")?;
                a.format = parse_format(&v).ok_or_else(|| format!("unknown format: {v} (see --formats)"))?;
                have_format = true;
            }
            "-d" | "--depth" => a.depth = Some(val("--depth")?),
            "-p" | "--pal" => a.pal = Some(PathBuf::from(val("--pal")?)),
            "--read-pal" => a.read_pal = Some(val("--read-pal")?),
            "-o" | "--out" => a.out = Some(PathBuf::from(val("--out")?)),
            "-n" | "--name" => a.name = Some(val("--name")?),
            "-r" | "--recursive" => a.recursive = true,
            "--dither" => a.dither = true,
            "--dither-method" => {
                let v = val("--dither-method")?;
                a.dither_method = DitherMethod::from_name(&v).ok_or_else(|| format!("--dither-method expects fs or bayer (got {v})"))?;
                a.dither = true;
            }
            "--scale" => {
                let p = val("--scale")?.trim_end_matches('%').parse::<u32>().ok().filter(|p| (1..=1000).contains(p)).ok_or("--scale expects a percentage from 1 to 1000")?;
                a.resize = Resize::Percent(p);
            }
            "--max-size" => {
                let n = val("--max-size")?.parse::<u32>().ok().filter(|n| *n >= 1).ok_or("--max-size expects a number of pixels >= 1")?;
                a.resize = Resize::MaxSide(n);
            }
            "--resample" => {
                let v = val("--resample")?;
                a.resample = Resample::from_name(&v).ok_or_else(|| format!("--resample expects smooth or pixel (got {v})"))?;
            }
            "--trim" => a.trim = true,
            "--sequence" => a.seq = true,
            "-q" | "--quiet" => a.quiet = true,
            "--direction" => a.dir = index(val("--direction")?, "--direction")?,
            "--frame" => a.frame = index(val("--frame")?, "--frame")?,
            "--quality" => {
                a.quality = val("--quality")?.parse::<u8>().ok().filter(|q| (1..=100).contains(q)).ok_or("quality must be 1-100")?;
            }
            "--png-level" => {
                let v = val("--png-level")?;
                a.png_level = PngLevel::from_name(&v).ok_or_else(|| format!("--png-level expects fast, normal or best (got {v})"))?;
            }
            "--bg" => {
                let v = val("--bg")?;
                let v = v.trim_start_matches('#');
                if v.len() != 6 {
                    return Err("--bg expects RRGGBB".into());
                }
                let n = u32::from_str_radix(v, 16).map_err(|_| "--bg expects RRGGBB")?;
                a.bg = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            }
            "--lang" => {
                let v = val("--lang")?;
                match crate::i18n::Lang::ALL.iter().find(|l| l.code() == v.to_lowercase()) {
                    Some(l) => crate::i18n::set_lang(*l),
                    None => return Err(format!("unknown language: {v}")),
                }
            }
            s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            _ => {
                if have_input {
                    return Err("only one input (file or folder) is accepted".into());
                }
                a.input = PathBuf::from(arg);
                have_input = true;
            }
        }
    }
    if !have_input {
        return Err("missing input file or folder".into());
    }
    if !have_format {
        return Err("missing --format".into());
    }
    Ok(a)
}

fn depth_from(spec: &str, pal: &Option<Arc<Palette>>) -> Result<DepthChoice, String> {
    Ok(match spec.to_lowercase().as_str() {
        "8" => DepthChoice::Indexed(PalChoice::Adaptive),
        "8f" => DepthChoice::Indexed(PalChoice::Fallout),
        "8p" => DepthChoice::Indexed(match pal {
            Some(p) => PalChoice::Custom(p.clone()),
            None => PalChoice::Create,
        }),
        "16" => DepthChoice::Rgb16,
        "24" => DepthChoice::Rgb24,
        "32" => DepthChoice::Rgba32,
        s => return Err(format!("unknown depth: {s} (8, 8f, 8p, 16, 24, 32)")),
    })
}

/// Tableau des formats et des profondeurs acceptées.
fn formats_table() -> String {
    let mut s = String::from("FORMAT  EXT   DEPTHS (-d)                         ANIMATION / DIRECTIONS\n");
    for f in Format::ALL {
        let c = formats::caps(f);
        let mut d: Vec<&str> = Vec::new();
        if c.indexed {
            if !c.no_adaptive {
                d.push("8");
            }
            d.push("8f");
            if c.custom_pal {
                d.push("8p");
            }
        }
        if c.d16 {
            d.push("16");
        }
        if c.d24 {
            d.push("24");
        }
        if c.d32 {
            d.push("32");
        }
        s += &format!(
            "{:<7} {:<5} {:<35} {}\n",
            f.name(),
            f.ext(),
            d.join(" "),
            if c.multi_frame { "all frames kept (frm/art: all directions)" } else { "single image (--frame/--direction/--sequence)" }
        );
    }
    s += "\nRead only (INPUT of convert / palette): ";
    s += &Format::READ_ONLY.iter().map(|f| f.name().to_string()).collect::<Vec<_>>().join(", ");
    s += ".\nReading also accepts .fr0-.fr5 (FRM), .jpeg/.jpe, .tiff, .targa, .pbm/.pgm/.pnm/.pam, .aseprite. Inputs may be inside .DAT archives (Fallout 1 and 2).\n";
    s
}

fn print_out(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
}

/// `extract <entrée> [-o dossier] [--overwrite] [-q]` : extrait des fichiers d'une archive .DAT, sans conversion.
fn run_extract(args: &[String]) -> i32 {
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "/?" | "-?")) {
        print_out(&usage());
        return 0;
    }
    let (mut input, mut out, mut overwrite, mut quiet) = (None, None, false, false);
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = it.next().map(PathBuf::from),
            "--overwrite" => overwrite = true,
            "-q" | "--quiet" => quiet = true,
            s if s.starts_with('-') => {
                eprintln!("error: unknown option: {s}");
                return 2;
            }
            _ => input = Some(PathBuf::from(a)),
        }
    }
    let Some(input) = input else {
        eprintln!("error: usage: extract <INPUT> [-o DIR] [--overwrite] [-q]");
        return 2;
    };
    let Some((archive, _)) = crate::dat::split(&input) else {
        eprintln!("error: INPUT must be a .DAT archive, or a folder or file inside one: {}", input.display());
        return 2;
    };
    let out = out.unwrap_or_else(|| PathBuf::from(archive.file_stem().unwrap_or_default()));
    match crate::dat::extract(&input, &out, overwrite) {
        Ok(r) => {
            if !quiet || r.skipped > 0 {
                println!("{} extracted, {} skipped (already present)  ->  {}", r.written, r.skipped, out.display());
            }
            0
        }
        Err(e) => {
            eprintln!("FAIL  {}  :  {e}", input.display());
            1
        }
    }
}

/// `palette <entrée> -f <format> [-o fichier|dossier] [--read-pal p]`.
fn run_palette(args: &[String]) -> i32 {
    use crate::palette_export::{self, PalFormat};
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "/?" | "-?")) {
        print_out(&usage());
        return 0;
    }
    let (mut input, mut fmt, mut out, mut read_pal) = (None, None, None, None);
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-f" | "--format" => fmt = it.next().cloned(),
            "-o" | "--out" => out = it.next().cloned(),
            "--read-pal" | "-p" | "--pal" => read_pal = it.next().cloned(),
            s if s.starts_with('-') => {
                eprintln!("error: unknown option: {s}");
                return 2;
            }
            _ => input = Some(PathBuf::from(a)),
        }
    }
    let (Some(input), Some(fmt)) = (input, fmt) else {
        eprintln!("error: usage: palette <INPUT> --format <pal|riff|act|gpl|kpl|hex> [-o FILE_OR_DIR]");
        return 2;
    };
    let Some(pf) = PalFormat::from_id(&fmt) else {
        eprintln!("error: unknown palette format: {fmt}");
        return 2;
    };
    let policy = match read_pal.as_deref() {
        None | Some("auto") => PalPolicy::Auto,
        Some("fallout") => PalPolicy::Fallout,
        Some(f) => match load_pal_file(Path::new(f)) {
            Ok(p) => PalPolicy::Custom(Arc::new(p), f.to_string()),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
    };
    let loaded = match crate::loader::load_file(&input, &policy) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("FAIL  {}  :  {e}", input.display());
            return 1;
        }
    };
    let stem = batch::stem_of(&input);
    let target = match out {
        Some(o) if !Path::new(&o).is_dir() && Path::new(&o).extension().is_some() => PathBuf::from(o),
        Some(o) => batch::unique_path(Path::new(&o), &stem, pf.ext(), Path::new("")),
        None => batch::unique_path(&crate::dat::real_parent(&input).unwrap_or_default(), &stem, pf.ext(), Path::new("")),
    };
    let name = target.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(stem);
    let result = palette_export::encode(pf, &loaded.pic, &name).map_err(|e| e.to_string()).and_then(|b| {
        if let Some(d) = target.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&target, b).map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => {
            println!("OK    {}  ->  {}", input.display(), target.display());
            0
        }
        Err(e) => {
            eprintln!("FAIL  {}  :  {e}", input.display());
            1
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let first = args.first().map(|s| s.as_str()).unwrap_or("");
    match first {
        "-V" | "--version" => {
            print_out(&format!(
                "{} {}
",
                exe_name(),
                env!("CARGO_PKG_VERSION")
            ));
            return 0;
        }
        "--formats" => {
            print_out(&formats_table());
            return 0;
        }
        "convert" => {}
        "palette" => return run_palette(args),
        "extract" => return run_extract(args),
        _ => {
            print_out(&usage());
            return 0;
        }
    }
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "/?" | "-?")) {
        print_out(&usage());
        return 0;
    }
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "error: {e}

Try: {} --help",
                exe_name()
            );
            return 2;
        }
    };
    let pal: Option<Arc<Palette>> = match &a.pal {
        Some(p) => match load_pal_file(p) {
            Ok(x) => Some(Arc::new(x)),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
        None => None,
    };
    let depth = match &a.depth {
        Some(d) => match depth_from(d, &pal) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
        None => default_depth(a.format),
    };
    let caps = formats::caps(a.format);
    if !depth_supported(&caps, &depth) {
        eprintln!("error: this depth is not available for {} (see --formats)", a.format.name());
        return 2;
    }
    // Palette utilisée pour LIRE les FRM.
    let policy = match a.read_pal.as_deref() {
        Some("auto") => PalPolicy::Auto,
        Some("fallout") => PalPolicy::Fallout,
        Some(file) => match load_pal_file(Path::new(file)) {
            Ok(p) => PalPolicy::Custom(Arc::new(p), file.rsplit(['\\', '/']).next().unwrap_or(file).to_string()),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
        None => match (&a.pal, &pal) {
            (Some(path), Some(p)) => PalPolicy::Custom(p.clone(), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            _ => PalPolicy::Auto,
        },
    };
    let (inputs, dir_mode, from_archive) = match batch::list_inputs(&a.input, a.recursive) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if inputs.is_empty() {
        eprintln!("nothing to convert: no supported image in {}", a.input.display());
        return 1;
    }
    if a.name.is_some() && dir_mode {
        eprintln!("error: --name can only be used in file mode");
        return 2;
    }
    // Dossier de sortie de base.
    let base_out: Option<PathBuf> = match (&a.out, from_archive) {
        (Some(o), _) => Some(o.clone()),
        (None, true) if dir_mode => {
            let stem = crate::dat::split(&a.input).map(|(arch, _)| batch::stem_of(&arch)).unwrap_or_else(|| "dat".into());
            Some(PathBuf::from(stem))
        }
        _ => None,
    };
    let job = Job {
        opts: ConvertOptions {
            format: a.format,
            depth,
            dither: a.dither,
            bg: a.bg,
            enc: EncodeOpts { jpeg_quality: a.quality, png_level: a.png_level },
            ops: ImageOps { resize: a.resize, resample: a.resample, trim: a.trim, dither_method: a.dither_method },
        },
        seq: a.seq,
        policy,
    };
    let sel = Selection { dir: a.dir, frame: a.frame };
    let total = inputs.len();
    let counters = Mutex::new((0usize, 0usize));
    let out_lock = Mutex::new(());
    inputs.par_iter().for_each(|(path, rel)| {
        let out_dir = match &base_out {
            Some(b) => b.join(rel),
            None => crate::dat::real_parent(path).unwrap_or_default(),
        };
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| batch::convert_path(path, &job, &out_dir, a.name.as_deref(), &sel)))
            .unwrap_or_else(|_| Err("internal error".into()));
        let _g = out_lock.lock().unwrap();
        let mut c = counters.lock().unwrap();
        match res {
            Ok(files) => {
                c.0 += 1;
                if !a.quiet {
                    for f in files {
                        println!("OK    {}  ->  {}", path.display(), f.display());
                    }
                }
            }
            Err(e) => {
                c.1 += 1;
                eprintln!("FAIL  {}  :  {e}", path.display());
            }
        }
    });
    let (ok, fail) = *counters.lock().unwrap();
    println!("{ok} / {total} converted, {fail} failed");
    let _ = std::io::stdout().flush();
    if fail > 0 { 1 } else { 0 }
}
