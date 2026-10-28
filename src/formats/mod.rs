//! Lecture / écriture des formats d'image.

pub mod art;
pub mod ase;
pub mod bmp;
pub mod dds;
pub mod frm;
pub mod gif;
pub mod jpeg;
pub mod ktx2;
pub mod layered;
pub mod palfile;
pub mod pcx;
pub mod pdn;
pub mod png;
pub mod pnm;
pub mod psd;
pub mod qoi;
pub mod rix;
pub mod tactics;
pub mod tga;
pub mod tiff;
pub mod webp;
pub mod xcf;

use crate::model::*;
use anyhow::{Result, anyhow, bail};
use std::path::Path;
use std::sync::Arc;

/// Contexte de décodage (palette à appliquer aux formats indexés sans palette intégrée).
#[derive(Clone)]
pub struct DecodeCtx {
    pub palette: Arc<Palette>,
    /// Origine de la palette, pour l'affichage dans les propriétés.
    pub palette_name: String,
    /// Extension du fichier en minuscules (distingue ACT et PAL).
    pub ext: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    Indexed,
    Rgb16,
    Rgb24,
    Rgba32,
}

#[derive(Clone, Debug)]
pub enum OutData {
    Indexed(Vec<u8>),
    Rgba(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct OutFrame {
    pub width: u32,
    pub height: u32,
    pub data: OutData,
    pub pos: (i32, i32),
    pub delay_ms: Option<u32>,
    /// Opacité de chaque pixel (0 à 255) pour les formats à palette qui la stockent à part (ZAR) ; `None` : celle de la palette.
    pub alpha: Option<Vec<u8>>,
}

/// Image prête à être écrite par un encodeur : les pixels et la palette sont déjà dans
/// le format de sortie voulu.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub format: Format,
    pub depth: Depth,
    pub frames: Vec<OutFrame>,
    pub directions: usize,
    pub frames_per_dir: usize,
    pub fps: u32,
    pub action_frame: u32,
    pub dir_shift: Vec<(i32, i32)>,
    /// Palette écrite dans le fichier. L'entrée d'alpha 0 est la couleur transparente.
    pub palette: Option<Palette>,
}

impl Prepared {
    pub fn first(&self) -> &OutFrame {
        &self.frames[0]
    }
    /// Index transparent de la palette de sortie (alpha 0), le cas échéant.
    pub fn transparent_index(&self) -> Option<u8> {
        let p = self.palette.as_ref()?;
        (0..256usize).find(|&i| p.used[i] && p.colors[i][3] == 0).map(|i| i as u8)
    }
}

/// Niveau de compression d'un PNG : le niveau « normal » peut prendre plusieurs secondes sur une grande photo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PngLevel {
    /// Rapide : fichier nettement plus gros sur une photo, quasi identique sur une illustration.
    Fast,
    #[default]
    Normal,
    /// Maximum : le plus lent, fichier un peu plus petit.
    Best,
}

impl PngLevel {
    pub const ALL: [PngLevel; 3] = [PngLevel::Fast, PngLevel::Normal, PngLevel::Best];

    /// Clé de traduction.
    pub fn key(self) -> &'static str {
        match self {
            PngLevel::Fast => "c.png_fast",
            PngLevel::Normal => "c.png_normal",
            PngLevel::Best => "c.png_best",
        }
    }

    /// Nom utilisé en ligne de commande.
    pub fn name(self) -> &'static str {
        match self {
            PngLevel::Fast => "fast",
            PngLevel::Normal => "normal",
            PngLevel::Best => "best",
        }
    }

    pub fn from_name(s: &str) -> Option<PngLevel> {
        PngLevel::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Debug)]
pub struct EncodeOpts {
    pub jpeg_quality: u8,
    pub png_level: PngLevel,
    /// KTX2 : compresser en BC7 (avec perte) au lieu d'écrire du RGBA8 sans perte.
    pub ktx2_bc7: bool,
}

impl Default for EncodeOpts {
    fn default() -> Self {
        EncodeOpts { jpeg_quality: 92, png_level: PngLevel::Normal, ktx2_bc7: false }
    }
}

/// Description des profondeurs acceptées par chaque format.
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    pub indexed: bool,
    pub d16: bool,
    pub d24: bool,
    pub d32: bool,
    /// Le format sait enregistrer un index (ou une entrée de palette) transparent.
    pub indexed_alpha: bool,
    /// Le format peut contenir plusieurs images (animation / directions).
    pub multi_frame: bool,
    /// L'index 0 est toujours la transparence (FRM, ART).
    pub index0_transparent: bool,
    /// Palette imposée à Fallout / PAL personnalisée uniquement (pas d'adaptative).
    pub no_adaptive: bool,
    /// Accepte une palette personnalisée (.PAL, existante ou créée) : réservé au FRM, dont la palette est externe.
    pub custom_pal: bool,
}

pub fn caps(f: Format) -> Caps {
    let base =
        Caps { indexed: false, d16: false, d24: false, d32: false, indexed_alpha: false, multi_frame: false, index0_transparent: false, no_adaptive: false, custom_pal: false };
    match f {
        Format::Art => Caps { indexed: true, indexed_alpha: true, multi_frame: true, index0_transparent: true, ..base },
        Format::Bmp => Caps { indexed: true, d16: true, d24: true, d32: true, ..base },
        Format::Frm => Caps { indexed: true, indexed_alpha: true, multi_frame: true, index0_transparent: true, no_adaptive: true, custom_pal: true, ..base },
        Format::Gif => Caps { indexed: true, indexed_alpha: true, multi_frame: true, ..base },
        Format::Jpeg => Caps { d24: true, d32: true, ..base },
        Format::Pcx => Caps { indexed: true, d24: true, d32: true, ..base },
        Format::Png => Caps { indexed: true, indexed_alpha: true, d16: true, d24: true, d32: true, ..base },
        Format::Rix => Caps { indexed: true, ..base },
        Format::Tiff => Caps { indexed: true, d16: true, d24: true, d32: true, ..base },
        Format::Tga => Caps { indexed: true, d16: true, d24: true, d32: true, ..base },
        Format::Webp => Caps { d24: true, d32: true, ..base },
        // Formats en lecture seule.
        Format::Psd | Format::Pdn | Format::Xcf | Format::Kra | Format::Ora => base,
        Format::Dds => Caps { d16: true, d24: true, d32: true, ..base },
        Format::Qoi | Format::Pnm | Format::Ktx2 => Caps { d24: true, d32: true, ..base },
        Format::Ase | Format::Palette | Format::Til | Format::Spr => base,
        // ZAR (Fallout Tactics) : image à palette 256 entrées avec transparence ; le format n'a pas d'autre profondeur.
        Format::Zar => Caps { indexed: true, indexed_alpha: true, ..base },
    }
}

/// Décode un fichier. `path` sert aux formats qui ont besoin de fichiers voisins (FRM + .PAL).
pub fn decode(format: Format, data: &[u8], ctx: &DecodeCtx) -> Result<Picture> {
    let mut pic = match format {
        Format::Art => art::decode(data),
        Format::Bmp => bmp::decode(data),
        Format::Frm => frm::decode(data, ctx),
        Format::Gif => gif::decode(data),
        Format::Jpeg => jpeg::decode(data),
        Format::Pcx => pcx::decode(data),
        Format::Png => png::decode(data),
        Format::Rix => rix::decode(data),
        Format::Tiff => tiff::decode(data),
        Format::Tga => tga::decode(data),
        Format::Webp => webp::decode(data),
        Format::Psd => psd::decode(data),
        Format::Pdn => pdn::decode(data),
        Format::Xcf => xcf::decode(data),
        Format::Kra => layered::decode_kra(data),
        Format::Ora => layered::decode_ora(data),
        Format::Dds => dds::decode(data),
        Format::Qoi => qoi::decode(data),
        Format::Ktx2 => ktx2::decode(data),
        Format::Pnm => pnm::decode(data),
        Format::Ase => ase::decode(data),
        Format::Palette => palfile::decode(data, &ctx.ext),
        Format::Zar => tactics::decode_zar(data),
        Format::Til => tactics::decode_til(data),
        Format::Spr => tactics::decode_spr(data),
    }?;
    finish(&mut pic);
    Ok(pic)
}

/// Détecte le format d'après le contenu (utile si l'extension est trompeuse).
pub fn sniff(data: &[u8]) -> Option<Format> {
    if data.len() < 12 {
        return None;
    }
    if &data[..2] == b"BM" {
        return Some(Format::Bmp);
    }
    if &data[..4] == b"GIF8" {
        return Some(Format::Gif);
    }
    if data[..3] == [0xFF, 0xD8, 0xFF] {
        return Some(Format::Jpeg);
    }
    if &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some(Format::Webp);
    }
    if data[..4] == [0x49, 0x49, 0x2A, 0x00] || data[..4] == [0x4D, 0x4D, 0x00, 0x2A] {
        return Some(Format::Tiff);
    }
    if &data[..4] == b"RIX3" {
        return Some(Format::Rix);
    }
    if data[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some(Format::Png);
    }
    if data.starts_with(b"<zar>") {
        return Some(Format::Zar);
    }
    if data.starts_with(b"<tile>") {
        return Some(Format::Til);
    }
    if data.starts_with(b"<sprite>") {
        return Some(Format::Spr);
    }
    if data.len() >= 12 && data[..12] == [0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some(Format::Ktx2);
    }
    if &data[..4] == b"qoif" {
        return Some(Format::Qoi);
    }
    if data.len() > 6 && data[4] == 0xE0 && data[5] == 0xA5 {
        return Some(Format::Ase);
    }
    if data.starts_with(b"GIMP Palette") || data.starts_with(b"JASC-PAL") || (&data[..4] == b"RIFF" && &data[8..12] == b"PAL ") {
        return Some(Format::Palette);
    }
    if &data[..4] == b"DDS " {
        return Some(Format::Dds);
    }
    if &data[..4] == b"8BPS" {
        return Some(Format::Psd);
    }
    if &data[..4] == b"PDN3" {
        return Some(Format::Pdn);
    }
    if &data[..9] == b"gimp xcf " {
        return Some(Format::Xcf);
    }
    if crate::zipread::Zip::is_zip(data)
        && let Ok(z) = crate::zipread::Zip::open(data)
    {
        let mime = z.read("mimetype").unwrap_or_default();
        if mime.starts_with(b"image/openraster") {
            return Some(Format::Ora);
        }
        if mime.starts_with(b"application/x-krita") {
            return Some(Format::Kra);
        }
    }
    if data[0] == 0x0A && data[2] == 1 && (data[1] <= 5) {
        return Some(Format::Pcx);
    }
    if data[0] == b'P' && (b'1'..=b'7').contains(&data[1]) && data[2].is_ascii_whitespace() {
        return Some(Format::Pnm);
    }
    None
}

pub fn encode(prep: &Prepared, opts: &EncodeOpts) -> Result<Vec<u8>> {
    if prep.frames.is_empty() {
        bail!("aucune image à écrire");
    }
    match prep.format {
        Format::Art => art::encode(prep),
        Format::Bmp => bmp::encode(prep),
        Format::Dds => dds::encode(prep),
        Format::Qoi => qoi::encode(prep),
        Format::Ktx2 => ktx2::encode(prep, opts),
        Format::Pnm => pnm::encode(prep),
        Format::Frm => frm::encode(prep),
        Format::Gif => gif::encode(prep),
        Format::Jpeg => jpeg::encode(prep, opts),
        Format::Pcx => pcx::encode(prep),
        Format::Png => png::encode(prep, opts),
        Format::Rix => rix::encode(prep),
        Format::Tiff => tiff::encode(prep),
        Format::Tga => tga::encode(prep),
        Format::Webp => webp::encode(prep),
        Format::Zar => tactics::encode_zar(prep),
        Format::Psd | Format::Pdn | Format::Xcf | Format::Kra | Format::Ora | Format::Ase | Format::Palette | Format::Til | Format::Spr => {
            bail!("{} : format en lecture seule", prep.format.name())
        }
    }
}

/// Calculs communs une fois l'image décodée : transparence et nombre de couleurs.
fn finish(pic: &mut Picture) {
    let mut seen: Vec<u64> = Vec::new();
    let mut unique = 0usize;
    let mut transparent = false;
    if let Some(pal) = &pic.palette {
        let mut used = [false; 256];
        for f in &pic.frames {
            if let Pixels::Indexed(idx) = &f.pixels {
                for &i in idx {
                    used[i as usize] = true;
                }
            }
        }
        unique = used.iter().filter(|u| **u).count();
        transparent = (0..256).any(|i| used[i] && pal.colors[i][3] < 255);
    }
    for f in &pic.frames {
        if let Pixels::Rgba(v) = &f.pixels {
            if pic.frames.len() > 8 && unique > 0 {
                break;
            }
            unique += crate::model::count_unique_rgb(v, &mut seen);
            if !transparent && v.chunks_exact(4).any(|p| p[3] < 255) {
                transparent = true;
            }
        }
    }
    pic.unique_colors = unique;
    pic.has_transparency = transparent;
}

pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Lecture binaire sécurisée
// ---------------------------------------------------------------------------

pub struct Rd<'a> {
    pub d: &'a [u8],
    pub pos: usize,
}

impl<'a> Rd<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Rd { d, pos: 0 }
    }
    pub fn at(d: &'a [u8], pos: usize) -> Self {
        Rd { d, pos }
    }
    pub fn left(&self) -> usize {
        self.d.len().saturating_sub(self.pos)
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| anyhow!("fichier tronqué"))?;
        if end > self.d.len() {
            bail!("fichier tronqué (offset {} + {} > {})", self.pos, n, self.d.len());
        }
        let s = &self.d[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn u16le(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u32le(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn i32le(&mut self) -> Result<i32> {
        Ok(self.u32le()? as i32)
    }
    pub fn u16be(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    pub fn i16be(&mut self) -> Result<i16> {
        Ok(self.u16be()? as i16)
    }
    pub fn u32be(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

pub(crate) fn put_u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
pub(crate) fn put_u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
pub(crate) fn put_u16be(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
pub(crate) fn put_u32be(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// Limite de sécurité sur la taille d'une image décodée.
/// Mémoire maximale (octets) des images d'une animation décodée : protège d'un fichier piégé annonçant des milliers d'images.
pub(crate) const MAX_ANIM_BYTES: usize = 1 << 30;

/// Refuse une animation de `frames` images de `frame_bytes` octets chacune si elle dépasse `MAX_ANIM_BYTES`.
pub(crate) fn check_anim_budget(frames: usize, frame_bytes: usize) -> Result<()> {
    if frames.saturating_mul(frame_bytes) > MAX_ANIM_BYTES {
        bail!("animation trop volumineuse ({frames} images de {} Mo)", frame_bytes >> 20);
    }
    Ok(())
}

pub(crate) fn check_dims(w: u64, h: u64) -> Result<()> {
    if w == 0 || h == 0 {
        bail!("dimensions nulles");
    }
    if w > 65535 * 2 || h > 65535 * 2 || w * h > 400_000_000 {
        bail!("dimensions excessives ({w}×{h})");
    }
    Ok(())
}

pub(crate) fn dpi_from_ppm(ppm: u32) -> String {
    format!("{:.0}", ppm as f64 * 0.0254)
}

/// Section EXIF (JPEG, TIFF, WebP) si des métadonnées sont présentes.
pub(crate) fn exif_section(data: &[u8]) -> Option<Section> {
    let ex = ::exif::Reader::new().read_from_container(&mut std::io::Cursor::new(data)).ok()?;
    let mut s = Section { title: "sec.exif".into(), items: Vec::new() };
    for f in ex.fields() {
        if f.ifd_num == ::exif::In::THUMBNAIL || f.tag == ::exif::Tag::MakerNote || f.tag == ::exif::Tag::UserComment && false {
            continue;
        }
        let mut v = f.display_value().with_unit(&ex).to_string();
        if v.chars().count() > 160 {
            v = v.chars().take(160).collect::<String>() + "…";
        }
        s.raw(&f.tag.to_string(), v);
        if s.items.len() >= 300 {
            break;
        }
    }
    if s.items.is_empty() { None } else { Some(s) }
}
