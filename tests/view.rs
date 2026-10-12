//! Calculs d'affichage : rotation / symétries, lecture d'un pixel, ancrage.

use fallout_image_browser::model::*;
use fallout_image_browser::view::*;

/// Image 3 × 2 dont chaque pixel est unique : (x, y) -> [x, y, 7, 255].
fn rgba_3x2() -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..2u8 {
        for x in 0..3u8 {
            v.extend_from_slice(&[x, y, 7, 255]);
        }
    }
    v
}

#[test]
fn transform_geometry() {
    let src = rgba_3x2();
    let px = |v: &[u8], w: usize, x: usize, y: usize| [v[(y * w + x) * 4], v[(y * w + x) * 4 + 1]];

    // Identité.
    let id = Transform::default();
    assert!(id.is_identity());
    assert_eq!(id.apply(&src, 3, 2), (src.clone(), 3, 2));

    // Quart de tour horaire : 3×2 -> 2×3 ; le coin haut-gauche d'origine passe en haut à droite.
    let mut t = Transform::default();
    t.rotate_right();
    let (v, w, h) = t.apply(&src, 3, 2);
    assert_eq!((w, h), (2, 3));
    assert_eq!(px(&v, w, 1, 0), [0, 0]);
    assert_eq!(px(&v, w, 0, 0), [0, 1]); // le coin bas-gauche d'origine passe en haut à gauche
    assert_eq!(px(&v, w, 1, 2), [2, 0]);

    // Quart de tour antihoraire : le coin haut-gauche d'origine passe en bas à gauche.
    let mut t = Transform::default();
    t.rotate_left();
    let (v, w, h) = t.apply(&src, 3, 2);
    assert_eq!((w, h), (2, 3));
    assert_eq!(px(&v, w, 0, 2), [0, 0]);

    // Demi-tour.
    let t = Transform { quarter_turns: 2, ..Default::default() };
    let (v, w, h) = t.apply(&src, 3, 2);
    assert_eq!((w, h), (3, 2));
    assert_eq!(px(&v, w, 0, 0), [2, 1]);

    // Symétries.
    let (v, w, _) = Transform { flip_h: true, ..Default::default() }.apply(&src, 3, 2);
    assert_eq!(px(&v, w, 0, 0), [2, 0]);
    let (v, w, _) = Transform { flip_v: true, ..Default::default() }.apply(&src, 3, 2);
    assert_eq!(px(&v, w, 0, 0), [0, 1]);

    // Quatre quarts de tour dans un sens ou dans l'autre ramènent à l'identité ; les identifiants de cache sont distincts.
    let mut t = Transform::default();
    for _ in 0..4 {
        t.rotate_right();
    }
    assert!(t.is_identity());
    t.rotate_left();
    t.rotate_right();
    assert!(t.is_identity());
    let mut ids = std::collections::HashSet::new();
    for q in 0..4 {
        for fh in [false, true] {
            for fv in [false, true] {
                ids.insert(Transform { quarter_turns: q, flip_h: fh, flip_v: fv }.id());
            }
        }
    }
    assert_eq!(ids.len(), 16);
}

/// `source_of` est l'inverse de `apply` pour les 16 combinaisons (c'est ce qui sert à lire le pixel sous la souris).
#[test]
fn source_of_matches_apply_for_every_combination() {
    let src = rgba_3x2();
    for q in 0..4u8 {
        for fh in [false, true] {
            for fv in [false, true] {
                let t = Transform { quarter_turns: q, flip_h: fh, flip_v: fv };
                let (v, tw, th) = t.apply(&src, 3, 2);
                assert_eq!(t.size(3, 2), (tw, th));
                for ty in 0..th {
                    for tx in 0..tw {
                        let (sx, sy) = t.source_of(tx, ty, 3, 2);
                        assert!(sx < 3 && sy < 2, "{t:?}");
                        assert_eq!(&v[(ty * tw + tx) * 4..(ty * tw + tx) * 4 + 4], &src[(sy * 3 + sx) * 4..(sy * 3 + sx) * 4 + 4], "{t:?} ({tx},{ty})");
                    }
                }
            }
        }
    }
}

fn indexed_pic() -> Picture {
    let mut pal = Palette::empty();
    for i in 0..4 {
        pal.colors[i] = [i as u8 * 60, 10, 200, 255];
        pal.used[i] = true;
    }
    pal.colors[0][3] = 0; // l'indice 0 est transparent
    // 2 × 2 : indices 0 1 / 2 3.
    Picture::single(Format::Frm, 2, 2, Pixels::Indexed(vec![0, 1, 2, 3]), Some(pal))
}

#[test]
fn pixel_info_reads_index_and_color() {
    let pic = indexed_pic();
    let p = pixel_info(&pic, 0, 0, 1, 1).unwrap();
    assert_eq!((p.x, p.y, p.index), (1, 1, Some(3)));
    assert_eq!(p.rgba, [180, 10, 200, 255]);
    assert_eq!(hex_color(p.rgba), "#B40AC8");
    // L'indice 0 est transparent : la couleur garde son alpha.
    let t = pixel_info(&pic, 0, 0, 0, 0).unwrap();
    assert_eq!((t.index, t.rgba[3]), (Some(0), 0));
    assert_eq!(hex_color(t.rgba), "#000AC800");
    // Hors du canevas.
    assert!(pixel_info(&pic, 0, 0, 2, 0).is_none() && pixel_info(&pic, 0, 0, -1, 0).is_none() && pixel_info(&pic, 0, 0, 0, 2).is_none());
}

#[test]
fn pixel_info_agrees_with_the_rendered_canvas_of_an_animation() {
    // Deux images de tailles et positions différentes dans une même direction : le canevas est leur union.
    let mut pal = Palette::empty();
    for i in 0..256 {
        pal.colors[i] = [i as u8, (i * 3) as u8, 9, 255];
        pal.used[i] = true;
    }
    let mut pic = Picture::new(Format::Frm);
    pic.directions = 1;
    pic.frames_per_dir = 2;
    pic.palette = Some(pal);
    pic.frames.push(Frame { width: 2, height: 2, pixels: Pixels::Indexed(vec![1, 2, 3, 4]), pos: (-1, -2), delay_ms: Some(100) });
    pic.frames.push(Frame { width: 3, height: 1, pixels: Pixels::Indexed(vec![5, 6, 7]), pos: (0, 0), delay_ms: Some(100) });
    for frame in 0..2 {
        let (cw, ch, rgba) = render_rgba(&pic, 0, frame);
        assert_eq!((cw as u32, ch as u32), canvas_size(&pic, 0));
        for y in 0..ch {
            for x in 0..cw {
                let info = pixel_info(&pic, 0, frame, x as i32, y as i32).unwrap();
                assert_eq!(info.rgba[..3], rgba[(y * cw + x) * 4..(y * cw + x) * 4 + 3], "frame {frame} ({x},{y})");
            }
        }
    }
    // Un pixel du canevas hors de l'image courante est transparent et sans indice.
    let outside = pixel_info(&pic, 0, 1, 0, 0).unwrap();
    assert_eq!((outside.rgba, outside.index), ([0; 4], None));
    // Ancrage : l'origine commune des positions (ici x0 = -1, y0 = -2).
    assert_eq!(anchor_in_canvas(&pic, 0), (1, 2));
}

#[test]
fn backgrounds_cycle_and_custom_uses_the_chosen_color() {
    let custom = [10, 20, 30];
    assert_eq!(Background::Checker.color(custom), None);
    assert_eq!(Background::Black.color(custom), Some([0, 0, 0]));
    assert_eq!(Background::White.color(custom), Some([255, 255, 255]));
    assert_eq!(Background::Custom.color(custom), Some(custom));
    // La touche B parcourt les cinq fonds puis revient au damier.
    let mut bg = Background::Checker;
    let mut seen = vec![bg];
    for _ in 0..5 {
        bg = bg.next();
        seen.push(bg);
    }
    assert_eq!(seen.last(), Some(&Background::Checker));
    assert!(seen.contains(&Background::Custom));
    // Les cinq premiers sont tous différents.
    for i in 0..5 {
        for j in i + 1..5 {
            assert_ne!(seen[i], seen[j]);
        }
    }
}

#[test]
fn colour_conversions_round_trip() {
    // Toutes les couleurs d'une grille reviennent identiques après RVB -> TSV -> RVB.
    for r in (0..=255).step_by(15) {
        for g in (0..=255).step_by(15) {
            for b in (0..=255).step_by(15) {
                let (h, s, v) = rgb_to_hsv([r as u8, g as u8, b as u8]);
                assert!((0.0..=1.0).contains(&h) && (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&v));
                assert_eq!(hsv_to_rgb(h, s, v), [r as u8, g as u8, b as u8], "({r},{g},{b})");
            }
        }
    }
    assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
    assert_eq!(hsv_to_rgb(1.0 / 3.0, 1.0, 1.0), [0, 255, 0]);
    assert_eq!(hsv_to_rgb(2.0 / 3.0, 1.0, 1.0), [0, 0, 255]);
    assert_eq!(hsv_to_rgb(0.5, 0.0, 0.5), [128, 128, 128]);
    assert_eq!(rgb_to_hsv([0, 0, 0]), (0.0, 0.0, 0.0));
}

#[test]
fn hex_colours_are_parsed_and_written() {
    assert_eq!(hex_rgb([0x56, 0x00, 0xD8]), "#5600D8");
    assert_eq!(parse_hex("#5600D8"), Some([0x56, 0x00, 0xD8]));
    assert_eq!(parse_hex("5600d8"), Some([0x56, 0x00, 0xD8]));
    assert_eq!(parse_hex("  #abc "), Some([0xAA, 0xBB, 0xCC]));
    for bad in ["", "#", "#12", "#12345", "#1234567", "#GG0000", "red", "#56 00D8"] {
        assert_eq!(parse_hex(bad), None, "{bad:?}");
    }
}

#[test]
fn recent_colours_keep_the_newest_first_without_duplicates() {
    let mut list = Vec::new();
    for c in [[1, 1, 1], [2, 2, 2], [3, 3, 3], [2, 2, 2], [4, 4, 4]] {
        push_recent(&mut list, c, 3);
    }
    assert_eq!(list, vec![[4, 4, 4], [2, 2, 2], [3, 3, 3]]);
}
