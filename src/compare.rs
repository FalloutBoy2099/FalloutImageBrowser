//! Comparaison de deux images : image de différences (en rouge) et statistiques.

/// Résultat de la comparaison de deux images RVBA.
pub struct Diff {
    /// Image des différences sur la zone commune (noir = identique, rouge = différent, d'autant plus vif que l'écart est grand).
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Nombre de pixels différents et nombre de pixels comparés.
    pub differing: usize,
    pub total: usize,
    /// Plus grand écart sur un canal (0 à 255).
    pub max_delta: u8,
    pub same_size: bool,
}

impl Diff {
    pub fn is_identical(&self) -> bool {
        self.same_size && self.differing == 0
    }

    /// Part des pixels comparés qui diffèrent, en pourcentage.
    pub fn percent(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.differing as f64 * 100.0 / self.total as f64 }
    }
}

/// Compare deux images RVBA (4 octets par pixel). Si les dimensions diffèrent, seule la zone commune (coin haut-gauche) est comparée.
pub fn diff(a: &[u8], aw: usize, ah: usize, b: &[u8], bw: usize, bh: usize) -> Diff {
    let (w, h) = (aw.min(bw), ah.min(bh));
    let mut rgba = vec![0u8; w * h * 4];
    let (mut differing, mut max_delta) = (0usize, 0u8);
    for y in 0..h {
        for x in 0..w {
            let pa = &a[(y * aw + x) * 4..][..4];
            let pb = &b[(y * bw + x) * 4..][..4];
            let o = (y * w + x) * 4;
            rgba[o + 3] = 255;
            // Deux pixels entièrement transparents sont identiques, quelle que soit leur couleur.
            if pa[3] == 0 && pb[3] == 0 {
                continue;
            }
            let delta = (0..4).map(|c| pa[c].abs_diff(pb[c])).max().unwrap_or(0);
            if delta > 0 {
                differing += 1;
                max_delta = max_delta.max(delta);
                rgba[o] = (96 + delta as u32 * 159 / 255) as u8;
            }
        }
    }
    Diff { rgba, width: w, height: h, differing, total: w * h, max_delta, same_size: aw == bw && ah == bh }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(c: [u8; 4], n: usize) -> Vec<u8> {
        (0..n).flat_map(|_| c).collect()
    }

    #[test]
    fn identical_images() {
        let a = px([10, 20, 30, 255], 6);
        let d = diff(&a, 3, 2, &a, 3, 2);
        assert!(d.is_identical());
        assert_eq!((d.differing, d.total, d.max_delta), (0, 6, 0));
        assert!(d.rgba.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    }

    #[test]
    fn counts_differing_pixels_and_largest_delta() {
        let a = px([0, 0, 0, 255], 4);
        let mut b = a.clone();
        b[0] = 40; // pixel 0 : +40 en rouge
        b[4 + 2] = 200; // pixel 1 : +200 en bleu
        let d = diff(&a, 2, 2, &b, 2, 2);
        assert_eq!((d.differing, d.max_delta), (2, 200));
        assert!((d.percent() - 50.0).abs() < 1e-9);
        assert!(d.rgba[0] > 96 && d.rgba[4] > d.rgba[0]);
    }

    #[test]
    fn transparent_pixels_ignore_their_colour() {
        let a = px([255, 0, 0, 0], 2);
        let b = px([0, 255, 0, 0], 2);
        assert!(diff(&a, 2, 1, &b, 2, 1).is_identical());
    }

    #[test]
    fn different_sizes_compare_the_common_area() {
        let a = px([1, 1, 1, 255], 6); // 3 x 2
        let b = px([1, 1, 1, 255], 4); // 2 x 2
        let d = diff(&a, 3, 2, &b, 2, 2);
        assert_eq!((d.width, d.height, d.total, d.differing), (2, 2, 4, 0));
        assert!(!d.same_size && !d.is_identical());
    }
}
