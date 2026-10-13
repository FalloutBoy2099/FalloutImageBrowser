//! Chemins rapides : comparaison de noms sans allocation, conversions en parallèle, table de couleurs partagée, préchargement borné.

use fallout_image_browser::convert::*;
use fallout_image_browser::formats::{self, DecodeCtx, EncodeOpts, PngLevel};
use fallout_image_browser::loader::{Loader, PalPolicy};
use fallout_image_browser::model::*;
use fallout_image_browser::palette::{self, Mapper};
use fallout_image_browser::tree::{FileEntry, SortKey, natural_cmp, ordered_files};
use fallout_image_browser::view::{Transform, box_downscale};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::Arc;

/// Ancienne implémentation (avec allocations), gardée comme référence.
fn reference_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ia, mut ib) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (mut na, mut nb) = (String::new(), String::new());
                while let Some(&c) = ia.peek() {
                    if c.is_ascii_digit() {
                        na.push(c);
                        ia.next();
                    } else {
                        break;
                    }
                }
                while let Some(&c) = ib.peek() {
                    if c.is_ascii_digit() {
                        nb.push(c);
                        ib.next();
                    } else {
                        break;
                    }
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ia.next();
                ib.next();
            }
        }
    }
}

#[test]
fn natural_order_matches_the_reference_implementation() {
    let fixed = [
        "", "a", "A", "img2", "img10", "img02", "IMG2.bmp", "img2.BMP", "x007", "x7", "x0", "x00", "é1", "É1", "été", "ete", "日本2", "日本10", "a1b2", "a1b10", "-5", "5-", "0",
        "00", "000a", "ÀÉ", "àé",
    ];
    for a in fixed {
        for b in fixed {
            assert_eq!(natural_cmp(a, b), reference_cmp(a, b), "{a:?} / {b:?}");
        }
    }
    // Noms engendrés : mélange de lettres, chiffres, zéros de tête et accents.
    let alphabet: Vec<char> = "aAbB0019_é日 .".chars().collect();
    let mut s = 0x2545_F491u32;
    let mut names = Vec::new();
    for _ in 0..300 {
        let mut n = String::new();
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        for k in 0..(s % 9) {
            n.push(alphabet[((s >> (k * 3)) as usize + k as usize * 7) % alphabet.len()]);
        }
        names.push(n);
    }
    for a in &names {
        for b in &names {
            assert_eq!(natural_cmp(a, b), reference_cmp(a, b), "{a:?} / {b:?}");
        }
    }
}

#[test]
fn descending_type_sort_is_the_exact_reverse() {
    let e = |n: &str, size: u64| FileEntry { path: PathBuf::from(n), name: n.to_string(), size, modified: None };
    let files = vec![e("b2.png", 1), e("a.BMP", 2), e("b10.png", 3), e("c.frm", 4), e("a2.bmp", 5), e("noext", 6)];
    let asc: Vec<&str> = ordered_files(&files, SortKey::Type, false, "").iter().map(|f| f.name.as_str()).collect();
    let mut desc: Vec<&str> = ordered_files(&files, SortKey::Type, true, "").iter().map(|f| f.name.as_str()).collect();
    desc.reverse();
    assert_eq!(asc, desc);
    assert_eq!(asc, ["noext", "a.BMP", "a2.bmp", "c.frm", "b2.png", "b10.png"]);
}

#[test]
fn box_downscale_averages_blocks_weighted_by_alpha() {
    // 3 × 3 opaque, facteur 2 : blocs de 2×2, 1×2, 2×1 et 1×1 aux bords.
    let mut img = Vec::new();
    for i in 0..9u8 {
        img.extend_from_slice(&[i * 20, 0, 255 - i * 20, 255]);
    }
    let (out, w, h) = box_downscale(&img, 3, 3, 2);
    assert_eq!((w, h, out.len()), (2, 2, 16));
    assert_eq!(&out[0..4], &[(20 + 60 + 80) / 4, 0, ((255 + 235 + 195 + 175) / 4) as u8, 255]);
    assert_eq!(&out[12..16], &[160, 0, 95, 255], "pixel du coin = seul pixel du bloc");
    // Un pixel transparent ne pèse pas dans la couleur, mais baisse l'alpha moyen.
    let img = [200, 0, 0, 255, 0, 200, 0, 0];
    let (out, w, h) = box_downscale(&img, 2, 1, 2);
    assert_eq!((w, h), (1, 1));
    assert_eq!(out, vec![200, 0, 0, 127]);
}

#[test]
fn transform_rotates_in_parallel_like_the_per_pixel_definition() {
    // Dimensions impaires et assez grandes pour que le travail soit réparti entre plusieurs threads.
    let (w, h) = (301usize, 257usize);
    let mut img = vec![0u8; w * h * 4];
    for (i, p) in img.chunks_exact_mut(4).enumerate() {
        p.copy_from_slice(&[(i % 251) as u8, (i / 251 % 251) as u8, (i % 7) as u8, 255]);
    }
    for turns in 0..4u8 {
        for (fh, fv) in [(false, false), (true, false), (false, true), (true, true)] {
            let t = Transform { quarter_turns: turns, flip_h: fh, flip_v: fv };
            let (out, tw, th) = t.apply(&img, w, h);
            assert_eq!(t.size(w, h), (tw, th));
            for ty in (0..th).step_by(13) {
                for tx in (0..tw).step_by(11) {
                    let (sx, sy) = t.source_of(tx, ty, w, h);
                    assert_eq!(out[(ty * tw + tx) * 4..][..4], img[(sy * w + sx) * 4..][..4], "{t:?} ({tx},{ty})");
                }
            }
        }
    }
    // Identité : aucune copie.
    let (same, tw, th) = Transform::default().apply_cow(Cow::Borrowed(&img[..]), w, h);
    assert!(matches!(same, Cow::Borrowed(_)));
    assert_eq!((tw, th), (w, h));
}

#[test]
fn large_indexed_images_convert_like_small_ones() {
    // Plus de 4 blocs de 65 536 pixels : chemin parallèle.
    let n = 300_000usize;
    let idx: Vec<u8> = (0..n).map(|i| (i * 7 % 256) as u8).collect();
    let pal = palette::fallout();
    let frame = Frame { width: 600, height: 500, pixels: Pixels::Indexed(idx.clone()), pos: (0, 0), delay_ms: None };
    let rgba = frame_to_rgba(&frame, Some(&pal));
    assert_eq!(rgba.len(), n * 4);
    for i in (0..n).step_by(997) {
        let c = idx[i] as usize;
        let want = if pal.used[c] { pal.colors[c] } else { [0, 0, 0, 255] };
        assert_eq!(rgba[i * 4..i * 4 + 4], want, "pixel {i}");
    }
    assert!(matches!(frame_to_rgba_cow(&frame, Some(&pal)), Cow::Owned(_)));
    let rgba_frame = Frame { width: 2, height: 1, pixels: Pixels::Rgba(vec![1, 2, 3, 4, 5, 6, 7, 8]), pos: (0, 0), delay_ms: None };
    assert!(matches!(frame_to_rgba_cow(&rgba_frame, None), Cow::Borrowed(_)));
}

#[test]
fn shared_mapper_is_reused_and_equal_to_a_fresh_one() {
    let pal = palette::fallout();
    let a = Mapper::shared(&pal, Some(0), true).unwrap();
    let b = Mapper::shared(&pal, Some(0), true).unwrap();
    assert!(Arc::ptr_eq(&a, &b), "la même palette réutilise la même table");
    let other = Mapper::shared(&pal, None, true).unwrap();
    assert!(!Arc::ptr_eq(&a, &other), "un autre réglage donne une autre table");
    let fresh = Mapper::new(&pal, Some(0), true).unwrap();
    for r in (0..=255u16).step_by(17) {
        for g in (0..=255u16).step_by(17) {
            for bl in (0..=255u16).step_by(17) {
                assert_eq!(a.nearest(r as u8, g as u8, bl as u8), fresh.nearest(r as u8, g as u8, bl as u8));
            }
        }
    }
}

#[test]
fn png_levels_all_decode_to_the_same_pixels() {
    let (w, h) = (96usize, 64usize);
    let mut v = vec![255u8; w * h * 4];
    for (i, p) in v.chunks_exact_mut(4).enumerate() {
        p[0] = (i % w * 2) as u8;
        p[1] = (i / w * 3) as u8;
        p[2] = ((i * 5) % 256) as u8;
    }
    let pic = Picture::single(Format::Bmp, w as u32, h as u32, Pixels::Rgba(v.clone()), None);
    let ctx = DecodeCtx { palette: palette::fallout(), palette_name: "F".into(), ext: String::new() };
    let mut sizes = Vec::new();
    for level in PngLevel::ALL {
        let opt = ConvertOptions {
            format: Format::Png,
            depth: DepthChoice::Rgba32,
            dither: false,
            bg: [0, 0, 0],
            enc: EncodeOpts { jpeg_quality: 92, png_level: level },
            ops: Default::default(),
        };
        let bytes = convert_to_bytes(&pic, &opt, &Selection::default()).unwrap();
        let back = formats::decode(Format::Png, &bytes, &ctx).unwrap();
        let Pixels::Rgba(px) = &back.frames[0].pixels else { panic!("RGBA attendu") };
        assert_eq!(px, &v, "{level:?}");
        sizes.push(bytes.len());
    }
    assert!(sizes[2] <= sizes[0], "le niveau maximal n'est pas plus gros que le rapide : {sizes:?}");
    assert_eq!(PngLevel::from_name("FAST"), Some(PngLevel::Fast));
    assert_eq!(PngLevel::from_name("best"), Some(PngLevel::Best));
    assert_eq!(PngLevel::from_name("zip"), None);
    assert_eq!(PngLevel::default(), PngLevel::Normal);
}

#[test]
fn loader_serves_urgent_requests_while_many_prefetches_are_queued() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_test_files").join("fallout").join("HMJMPSAA.FRM");
    if !src.is_file() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("fib_prefetch_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths: Vec<PathBuf> = (0..30)
        .map(|i| {
            let p = dir.join(format!("copy{i:02}.frm"));
            std::fs::copy(&src, &p).unwrap();
            p
        })
        .collect();
    let loader = Loader::new(eframe::egui::Context::default());
    let collect = |until: &dyn Fn(&std::collections::HashSet<PathBuf>) -> bool| {
        let mut got = std::collections::HashSet::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !until(&got) && std::time::Instant::now() < deadline {
            if let Ok(d) = loader.rx.recv_timeout(std::time::Duration::from_millis(200)) {
                assert!(d.result.is_ok(), "{}: {:?}", d.path.display(), d.result.err());
                got.insert(d.path);
            }
        }
        got
    };
    // Une demande urgente passe devant les préchargements et reçoit sa réponse.
    loader.prefetch(paths[1..].to_vec(), &PalPolicy::Fallout);
    loader.request(paths[0].clone(), PalPolicy::Fallout);
    let got = collect(&|g| g.contains(&paths[0]));
    assert!(got.contains(&paths[0]));
    // Les compteurs de préchargement restent justes : une nouvelle vague de 29 préchargements va bien jusqu'au bout.
    loader.prefetch(paths[1..].to_vec(), &PalPolicy::Fallout);
    let got = collect(&|g| paths[1..].iter().all(|p| g.contains(p)));
    assert!(paths[1..].iter().all(|p| got.contains(p)), "{} / {} reçus", got.len(), paths.len() - 1);
    let _ = std::fs::remove_dir_all(&dir);
}
