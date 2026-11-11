//! Animation de palette (« color cycling ») des images Fallout : certaines plages de la palette (slime, écrans, feux, rivage)
//! tournent en boucle dans le jeu, ce qui anime l'eau, les moniteurs ou les flammes sans changer les pixels de l'image.
//!
//! Les plages et les périodes sont celles du moteur de Fallout 2 telles qu'on les connaît (valeurs approximatives, faciles à
//! ajuster ici).

use crate::model::{Format, Palette, Picture, Pixels};

/// Une plage d'index qui tourne : de `first` à `last` inclus, d'un cran toutes les `period_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cycle {
    pub first: usize,
    pub last: usize,
    pub period_ms: u64,
    /// Sens inverse (le rivage tourne dans l'autre sens).
    pub reverse: bool,
}

impl Cycle {
    pub const fn len(&self) -> usize {
        self.last - self.first + 1
    }

    pub const fn is_empty(&self) -> bool {
        self.last < self.first
    }
}

pub const CYCLES: [Cycle; 5] = [
    Cycle { first: 229, last: 232, period_ms: 200, reverse: false }, // slime
    Cycle { first: 233, last: 237, period_ms: 100, reverse: false }, // moniteurs
    Cycle { first: 238, last: 242, period_ms: 200, reverse: false }, // feu lent
    Cycle { first: 243, last: 247, period_ms: 142, reverse: false }, // feu rapide
    Cycle { first: 248, last: 253, period_ms: 83, reverse: true },   // rivage
];

/// Cran de chaque plage à l'instant `now_ms`.
pub fn steps(now_ms: u64) -> [usize; CYCLES.len()] {
    let mut s = [0usize; CYCLES.len()];
    for (i, c) in CYCLES.iter().enumerate() {
        s[i] = ((now_ms / c.period_ms) % c.len() as u64) as usize;
    }
    s
}

/// Empreinte des crans : change quand (et seulement quand) l'image doit être redessinée.
pub fn phase(now_ms: u64) -> u64 {
    steps(now_ms).iter().fold(0u64, |acc, &s| acc * 8 + s as u64)
}

/// Délai avant le prochain changement de cran (pour ne redessiner que si nécessaire).
pub fn next_change_ms(now_ms: u64) -> u64 {
    CYCLES.iter().map(|c| c.period_ms - now_ms % c.period_ms).min().unwrap_or(100)
}

/// Palette dont les plages ont tourné de `steps` crans.
pub fn rotated(base: &Palette, steps: &[usize; CYCLES.len()]) -> Palette {
    let mut p = base.clone();
    for (c, &st) in CYCLES.iter().zip(steps) {
        let n = c.len();
        for k in 0..n {
            let from = if c.reverse { (k + n - st % n) % n } else { (k + st) % n };
            p.colors[c.first + k] = base.colors[c.first + from];
            p.used[c.first + k] = base.used[c.first + from];
        }
    }
    p
}

/// Couleurs de repli de chaque plage (approximatives) : la palette `default.pal` fournie avec l'application marque ces index
/// comme inutilisés, alors que la `color.pal` du jeu les définit.
const FALLBACK: [&[[u8; 3]]; 5] = [
    &[[0, 108, 0], [11, 115, 7], [27, 123, 15], [43, 131, 27]],
    &[[107, 107, 111], [99, 103, 127], [87, 107, 143], [0, 147, 163], [107, 187, 255]],
    &[[255, 0, 0], [215, 0, 0], [147, 43, 11], [255, 119, 0], [255, 59, 0]],
    &[[71, 0, 0], [123, 0, 0], [179, 0, 0], [123, 0, 0], [71, 0, 0]],
    &[[83, 63, 43], [75, 59, 43], [67, 51, 39], [63, 47, 39], [55, 43, 35], [51, 39, 31]],
];

/// Palette dont les plages animées sont définies : celles qui sont entièrement inutilisées reçoivent les couleurs de repli.
pub fn with_defaults(base: &Palette) -> Palette {
    let mut p = base.clone();
    for (c, fb) in CYCLES.iter().zip(FALLBACK) {
        if (c.first..=c.last).all(|i| !base.used[i]) {
            for (k, rgb) in fb.iter().enumerate() {
                p.colors[c.first + k] = [rgb[0], rgb[1], rgb[2], 255];
                p.used[c.first + k] = true;
            }
        }
    }
    p
}

/// Palette à l'instant `now_ms`.
pub fn cycled(base: &Palette, now_ms: u64) -> Palette {
    rotated(&with_defaults(base), &steps(now_ms))
}

/// La palette contient-elle (ou reçoit-elle par défaut) des couleurs à animer ?
pub fn has_cycling(p: &Palette) -> bool {
    let p = with_defaults(p);
    CYCLES.iter().any(|c| (c.first..=c.last).any(|i| p.colors[i] != p.colors[c.first]))
}

/// Une image indexée utilise-t-elle ces plages ? Seules comptent celles réellement présentes dans les pixels.
pub fn uses_cycling(pic: &Picture) -> bool {
    // Seuls les formats Fallout utilisent ces index comme couleurs animées ; un PNG / GIF 8 bits quelconque n'est pas concerné.
    if !matches!(pic.format, Format::Frm | Format::Rix) {
        return false;
    }
    let Some(pal) = &pic.palette else { return false };
    if !has_cycling(pal) {
        return false;
    }
    pic.frames.iter().any(|f| match &f.pixels {
        Pixels::Indexed(v) => v.iter().any(|&i| (229..=253).contains(&i)),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        let mut p = Palette::empty();
        for i in 0..256 {
            p.colors[i] = [i as u8, 0, 0, 255];
            p.used[i] = true;
        }
        p
    }

    #[test]
    fn step_zero_is_identity() {
        let p = pal();
        assert_eq!(rotated(&p, &[0; 5]), p);
    }

    #[test]
    fn rotation_moves_colours_inside_the_range_only() {
        let p = pal();
        let r = rotated(&p, &[1, 0, 0, 0, 0]);
        assert_eq!(r.colors[229][0], 230);
        assert_eq!(r.colors[232][0], 229);
        assert_eq!(r.colors[228], p.colors[228]);
        assert_eq!(r.colors[233], p.colors[233]);
    }

    #[test]
    fn full_turn_comes_back() {
        let p = pal();
        let n = CYCLES.map(|c| c.len());
        assert_eq!(rotated(&p, &n), p);
    }

    #[test]
    fn reverse_range_turns_the_other_way() {
        let p = pal();
        let r = rotated(&p, &[0, 0, 0, 0, 1]);
        assert_eq!(r.colors[248][0], 253);
        assert_eq!(r.colors[249][0], 248);
    }

    #[test]
    fn unused_ranges_get_fallback_colours() {
        let mut p = pal();
        for i in 229..=253 {
            p.used[i] = false;
            p.colors[i] = [0, 0, 0, 255];
        }
        assert!(has_cycling(&p));
        let q = with_defaults(&p);
        assert!((229..=253).all(|i| q.used[i]));
        // Une plage définie par la palette n'est pas touchée.
        let r = with_defaults(&pal());
        assert_eq!(r, pal());
    }

    #[test]
    fn only_fallout_formats_cycle() {
        use crate::model::Frame;
        let mut p = pal();
        for i in 229..=253 {
            p.colors[i] = [i as u8, 0, 0, 255];
        }
        for (fmt, want) in [(Format::Frm, true), (Format::Rix, true), (Format::Png, false), (Format::Gif, false), (Format::Bmp, false)] {
            let mut pic = Picture::new(fmt);
            pic.palette = Some(p.clone());
            pic.frames.push(Frame { width: 1, height: 1, pixels: Pixels::Indexed(vec![230]), pos: (0, 0), delay_ms: None });
            assert_eq!(uses_cycling(&pic), want, "{fmt:?}");
        }
    }

    #[test]
    fn phase_changes_with_time() {
        assert_ne!(phase(0), phase(90));
        assert!(next_change_ms(0) > 0);
    }
}
