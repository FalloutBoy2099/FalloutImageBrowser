use fallout_image_browser::cli;
use fallout_image_browser::formats::{self, DecodeCtx};
use fallout_image_browser::model::*;
use fallout_image_browser::palette;
use fallout_image_browser::palette_export::{PalFormat, encode};

fn ctx() -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: String::new() }
}

/// Fichiers d'exemple versionnés avec le projet (dossier `_test_files`).
fn sample(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join(rel)
}

fn indexed() -> Picture {
    let data = std::fs::read(sample("fallout/HMJMPSAA.FRM")).expect("_test_files/fallout/HMJMPSAA.FRM");
    formats::decode(Format::Frm, &data, &ctx()).unwrap()
}

fn truecolor() -> Picture {
    let (w, h) = (40usize, 30usize);
    let mut v = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            v[o] = (x * 6) as u8;
            v[o + 1] = (y * 8) as u8;
            v[o + 2] = ((x * y) % 256) as u8;
        }
    }
    Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(v), None)
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

#[test]
fn every_format_from_indexed_and_truecolor() {
    for pic in [indexed(), truecolor()] {
        // Fallout PAL : 768 octets 6 bits, relisible.
        let b = encode(PalFormat::FalloutPal, &pic, "t").unwrap();
        assert_eq!(b.len(), 768);
        assert!(b.iter().all(|&v| v <= 63));
        let p = palette::parse_pal(&b).unwrap();
        assert!(p.count_used() > 1);
        // RIFF PAL
        let b = encode(PalFormat::RiffPal, &pic, "t").unwrap();
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(&b[8..16], b"PAL data");
        assert_eq!(u32le(&b, 4) as usize, b.len() - 8);
        let n = u16::from_le_bytes([b[22], b[23]]) as usize;
        assert_eq!(u32le(&b, 16) as usize, 4 + 4 * n);
        assert_eq!(b.len(), 24 + 4 * n);
        assert!((2..=256).contains(&n));
        // ACT
        let b = encode(PalFormat::Act, &pic, "t").unwrap();
        assert!(b.len() == 768 || b.len() == 772);
        // GPL
        let t = String::from_utf8(encode(PalFormat::Gpl, &pic, "mon nom").unwrap()).unwrap();
        assert!(t.starts_with("GIMP Palette\nName: mon nom\nColumns: 16\n#\n"));
        assert_eq!(t.lines().count() - 4, n);
        // HEX
        let t = String::from_utf8(encode(PalFormat::Hex, &pic, "t").unwrap()).unwrap();
        assert_eq!(t.lines().count(), n);
        assert!(t.lines().all(|l| l.len() == 6 && l.chars().all(|c| c.is_ascii_hexdigit())));
        // KPL : archive zip valide, mimetype en premier.
        let b = encode(PalFormat::Kpl, &pic, "t & <u>").unwrap();
        assert_eq!(&b[..4], b"PK\x03\x04");
        assert_eq!(&b[30..38], b"mimetype");
        assert_eq!(&b[38..65], b"application/x-krita-palette");
        let eocd = b.len() - 22;
        assert_eq!(&b[eocd..eocd + 4], b"PK\x05\x06");
        assert_eq!(u16::from_le_bytes([b[eocd + 10], b[eocd + 11]]), 3);
        let cd = u32le(&b, eocd + 16) as usize;
        assert_eq!(&b[cd..cd + 4], b"PK");
        let (nl, ex, cm) = (u16::from_le_bytes([b[cd + 28], b[cd + 29]]) as usize, u16::from_le_bytes([b[cd + 30], b[cd + 31]]), u16::from_le_bytes([b[cd + 32], b[cd + 33]]));
        assert_eq!((&b[cd + 46..cd + 46 + nl], ex, cm), (&b"mimetype"[..], 0, 0));
        assert!(String::from_utf8_lossy(&b).contains("t &amp; &lt;u&gt;"));
    }
}

#[test]
fn indexed_palette_keeps_indices_in_act() {
    let pic = indexed();
    let b = encode(PalFormat::Act, &pic, "t").unwrap();
    let pal = pic.palette.as_ref().unwrap();
    for i in [1usize, 2, 100] {
        if pal.used[i] {
            assert_eq!(&b[i * 3..i * 3 + 3], &pal.colors[i][..3]);
        }
    }
    // Index 0 transparent signalé dans l'en-queue (FRM : couleur d'index 0 transparente).
    if b.len() == 772 {
        assert_eq!(u16::from_be_bytes([b[770], b[771]]), 0);
    }
}

#[test]
fn cli_palette_subcommand() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let dir = std::env::temp_dir().join(format!("fib_pal_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = sample("fallout/HMJMPSAA.FRM");
    let src = src.to_str().unwrap();
    let d = dir.to_str().unwrap();
    for f in ["pal", "riff", "act", "gpl", "kpl", "hex"] {
        assert_eq!(cli::run(&s(&["palette", src, "-f", f, "-o", d])), 0, "{f}");
    }
    assert!(dir.join("HMJMPSAA.kpl").exists() && dir.join("HMJMPSAA.hex").exists() && dir.join("HMJMPSAA.act").exists());
    assert_eq!(cli::run(&s(&["palette", src, "-f", "nope"])), 2);
    assert!(std::fs::read(dir.join("HMJMPSAA.act")).map(|b| b.len() >= 768).unwrap());
    // Dossier de test conservé pour inspection manuelle du .kpl.
    println!("{}", dir.display());
}

fn ctx_ext(ext: &str) -> DecodeCtx {
    DecodeCtx { palette: palette::fallout(), palette_name: "Fallout".into(), ext: ext.into() }
}

/// Chaque format de palette écrit doit pouvoir être relu (affichage en nuancier) avec les mêmes couleurs.
#[test]
fn palette_files_are_readable_as_swatches() {
    for pic in [indexed(), truecolor()] {
        for f in PalFormat::ALL {
            let src = fallout_image_browser::palette_export::extract(&pic, f == PalFormat::FalloutPal);
            let bytes = encode(f, &pic, "essai").unwrap();
            let sw = formats::decode(Format::Palette, &bytes, &ctx_ext(f.ext())).unwrap_or_else(|e| panic!("{} : {e}", f.label()));
            let colors = sw.colors.as_ref().expect("palette lue");
            assert!(colors.count_used() >= 2, "{}", f.label());
            // Aperçu RVBA en cases de 24 px, 16 colonnes.
            assert_eq!(sw.frames[0].width, 16 * 24);
            assert!(sw.frames[0].height.is_multiple_of(24));
            // Les couleurs lues sont celles de la palette extraite (RVB). Formats « à indices » (PAL Fallout, ACT) : mêmes
            // indices ; autres formats : liste séquentielle des couleurs non transparentes.
            let tol = if f == PalFormat::FalloutPal { 3 } else { 1 };
            let seq_src: Vec<[u8; 3]> = (0..256)
                .filter(|&i| src.palette.used[i] && src.palette.colors[i][3] > 0)
                .map(|i| [src.palette.colors[i][0], src.palette.colors[i][1], src.palette.colors[i][2]])
                .collect();
            let seq_read: Vec<[u8; 3]> = (0..256)
                .filter(|&i| colors.used[i] && colors.colors[i][3] > 0 || (f == PalFormat::FalloutPal && i == 0 && colors.used[0]))
                .map(|i| [colors.colors[i][0], colors.colors[i][1], colors.colors[i][2]])
                .collect();
            if matches!(f, PalFormat::FalloutPal | PalFormat::Act) {
                let mut checked = 0;
                for i in 1..256 {
                    if src.palette.used[i] && colors.used[i] {
                        for c in 0..3 {
                            assert!((src.palette.colors[i][c] as i32 - colors.colors[i][c] as i32).abs() <= tol, "{} entrée {i}", f.label());
                        }
                        checked += 1;
                    }
                }
                assert!(checked > 0, "{}", f.label());
            } else {
                // Un fichier sans index 0 réservé : les couleurs lues commencent par la première couleur utilisable.
                let n = seq_src.len().min(seq_read.len());
                assert!(n > 1, "{}", f.label());
                for k in 0..n {
                    for c in 0..3 {
                        assert!((seq_src[k][c] as i32 - seq_read[k][c] as i32).abs() <= tol, "{} couleur {k}", f.label());
                    }
                }
            }
            // Et on peut la reconvertir vers un autre format.
            assert!(encode(PalFormat::Gpl, &sw, "x").unwrap().len() > 20, "{}", f.label());
        }
    }
}

#[test]
fn jasc_and_hex_and_act_text_palettes() {
    let jasc = b"JASC-PAL\r\n0100\r\n3\r\n255 0 0\r\n0 255 0\r\n0 0 255\r\n";
    let p = formats::decode(Format::Palette, jasc, &ctx_ext("pal")).unwrap();
    assert_eq!(p.colors.as_ref().unwrap().count_used(), 3);
    assert_eq!(&p.colors.as_ref().unwrap().colors[1][..3], &[0, 255, 0]);
    let hex = b"ff0000\n00ff00\n0000ff\n#ffff00\n";
    let p = formats::decode(Format::Palette, hex, &ctx_ext("hex")).unwrap();
    assert_eq!(p.colors.as_ref().unwrap().count_used(), 4);
    assert_eq!(&p.colors.as_ref().unwrap().colors[3][..3], &[255, 255, 0]);
    assert_eq!(formats::sniff(jasc), Some(Format::Palette));
}
