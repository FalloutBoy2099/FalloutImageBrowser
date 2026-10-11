//! Extraction de la palette d'une image vers des fichiers de palette : PAL (Fallout), PAL (RIFF Microsoft),
//! ACT (Adobe), GPL (GIMP), KPL (Krita) et HEX.

use crate::model::*;
use crate::palette;
use anyhow::{Result, bail};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PalFormat {
    FalloutPal,
    RiffPal,
    Act,
    Gpl,
    Kpl,
    Hex,
}

impl PalFormat {
    pub const ALL: [PalFormat; 6] =
        [PalFormat::FalloutPal, PalFormat::RiffPal, PalFormat::Act, PalFormat::Gpl, PalFormat::Kpl, PalFormat::Hex];

    pub fn ext(self) -> &'static str {
        match self {
            PalFormat::FalloutPal | PalFormat::RiffPal => "pal",
            PalFormat::Act => "act",
            PalFormat::Gpl => "gpl",
            PalFormat::Kpl => "kpl",
            PalFormat::Hex => "hex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PalFormat::FalloutPal => ".PAL (Fallout Palette)",
            PalFormat::RiffPal => ".PAL (Microsoft RIFF Palette)",
            PalFormat::Act => ".ACT (Adobe Color Table)",
            PalFormat::Gpl => ".GPL (GIMP Palette)",
            PalFormat::Kpl => ".KPL (Krita Palette)",
            PalFormat::Hex => ".HEX (Hex Color Palette)",
        }
    }

    /// Identifiant stable (réglages, ligne de commande).
    pub fn id(self) -> &'static str {
        match self {
            PalFormat::FalloutPal => "pal",
            PalFormat::RiffPal => "riff",
            PalFormat::Act => "act",
            PalFormat::Gpl => "gpl",
            PalFormat::Kpl => "kpl",
            PalFormat::Hex => "hex",
        }
    }

    pub fn from_id(s: &str) -> Option<PalFormat> {
        PalFormat::ALL.iter().copied().find(|f| f.id() == s.to_ascii_lowercase())
    }
}

/// Palette extraite : 256 entrées (`used` = présente), `indexed` si l'image est déjà indexée (indices conservés).
pub struct Extracted {
    pub palette: Palette,
    pub indexed: bool,
}

pub fn extract(pic: &Picture, reserve_index0: bool) -> Extracted {
    // Fichier de palette affiché en nuancier : la palette lue est la source (indices conservés).
    if let Some(c) = &pic.colors {
        return Extracted { palette: c.clone(), indexed: true };
    }
    let indexed = pic.palette.is_some() && pic.frames.iter().all(|f| matches!(f.pixels, Pixels::Indexed(_)));
    if indexed {
        return Extracted { palette: pic.palette.clone().unwrap(), indexed: true };
    }
    let rgbas: Vec<Vec<u8>> = pic.frames.iter().map(|f| frame_to_rgba(f, pic.palette.as_ref())).collect();
    let refs: Vec<&[u8]> = rgbas.iter().map(|v| &v[..]).collect();
    Extracted { palette: palette::adaptive_palette(&refs, reserve_index0, [0, 0, 0]), indexed: false }
}

impl Extracted {
    /// Couleurs utilisables (hors entrées inutilisées ou transparentes), dans l'ordre des indices.
    fn colors(&self) -> Vec<(usize, [u8; 3])> {
        (0..256)
            .filter(|&i| self.palette.used[i] && self.palette.colors[i][3] > 0)
            .map(|i| (i, [self.palette.colors[i][0], self.palette.colors[i][1], self.palette.colors[i][2]]))
            .collect()
    }
}

pub fn encode(fmt: PalFormat, pic: &Picture, name: &str) -> Result<Vec<u8>> {
    let ex = extract(pic, fmt == PalFormat::FalloutPal);
    let colors = ex.colors();
    if colors.is_empty() {
        bail!("aucune couleur à extraire");
    }
    Ok(match fmt {
        PalFormat::FalloutPal => palette::write_pal(&ex.palette),
        PalFormat::RiffPal => riff(&colors),
        PalFormat::Act => act(&ex),
        PalFormat::Gpl => gpl(&colors, name, ex.indexed),
        PalFormat::Kpl => kpl(&colors, name, ex.indexed),
        PalFormat::Hex => colors.iter().map(|(_, c)| format!("{:02x}{:02x}{:02x}\n", c[0], c[1], c[2])).collect::<String>().into_bytes(),
    })
}

/// RIFF PAL : « RIFF » taille « PAL » « data » { version 0x0300, nombre, (R, V, B, 0)* }.
fn riff(colors: &[(usize, [u8; 3])]) -> Vec<u8> {
    let n = colors.len().min(256);
    let data_len = 4 + 4 * n;
    let mut o = Vec::with_capacity(24 + 4 * n);
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&((4 + 8 + data_len) as u32).to_le_bytes());
    o.extend_from_slice(b"PAL ");
    o.extend_from_slice(b"data");
    o.extend_from_slice(&(data_len as u32).to_le_bytes());
    o.extend_from_slice(&0x0300u16.to_le_bytes());
    o.extend_from_slice(&(n as u16).to_le_bytes());
    for (_, c) in colors.iter().take(256) {
        o.extend_from_slice(&[c[0], c[1], c[2], 0]);
    }
    o
}

/// ACT : 256 × RVB ; si moins de 256 couleurs (ou transparence), 4 octets finaux : nombre de couleurs et index transparent (big-endian).
fn act(ex: &Extracted) -> Vec<u8> {
    let mut o = vec![0u8; 768];
    let mut last = 0usize;
    let mut transparent: Option<usize> = None;
    if ex.indexed {
        for i in 0..256 {
            if ex.palette.used[i] {
                let c = ex.palette.colors[i];
                o[i * 3..i * 3 + 3].copy_from_slice(&c[..3]);
                last = i + 1;
                if c[3] == 0 && transparent.is_none() {
                    transparent = Some(i);
                }
            }
        }
    } else {
        for (k, (_, c)) in ex.colors().iter().take(256).enumerate() {
            o[k * 3..k * 3 + 3].copy_from_slice(c);
            last = k + 1;
        }
    }
    if last < 256 || transparent.is_some() {
        o.extend_from_slice(&(last as u16).to_be_bytes());
        o.extend_from_slice(&(transparent.map(|t| t as u16).unwrap_or(0xFFFF)).to_be_bytes());
    }
    o
}

fn gpl(colors: &[(usize, [u8; 3])], name: &str, indexed: bool) -> Vec<u8> {
    let mut s = format!("GIMP Palette\nName: {name}\nColumns: 16\n#\n");
    for (k, (i, c)) in colors.iter().enumerate() {
        let label = if indexed { *i } else { k };
        s += &format!("{:3} {:3} {:3}\tIndex {label}\n", c[0], c[1], c[2]);
    }
    s.into_bytes()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// Archive zip « stockée » (sans compression) ; le premier fichier doit être `mimetype`.
pub fn zip_store(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        // Date/heure DOS fixes (1er janvier 2024, 00:00).
        let (time, date) = (0u16, 0x5821u16);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, commentaire, disque, attributs internes
        central.extend_from_slice(&0u32.to_le_bytes()); // attributs externes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Krita : archive zip { mimetype, colorset.xml, profiles.xml }.
fn kpl(colors: &[(usize, [u8; 3])], name: &str, indexed: bool) -> Vec<u8> {
    let columns = 16usize;
    let rows = colors.len().div_ceil(columns);
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x += &format!("<Colorset name=\"{}\" comment=\"\" columns=\"{columns}\" rows=\"{rows}\" readonly=\"false\" version=\"1.0\">\n", xml_escape(name));
    for (k, (i, c)) in colors.iter().enumerate() {
        let label = if indexed { *i } else { k };
        x += &format!(
            " <ColorSetEntry name=\"Index {label}\" id=\"{label}\" bitdepth=\"U8\" spot=\"false\">\n  <RGB space=\"sRGB-elle-V2-srgbtrc.icc\" r=\"{:.6}\" g=\"{:.6}\" b=\"{:.6}\"/>\n  <Position row=\"{}\" column=\"{}\"/>\n </ColorSetEntry>\n",
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
            k / columns,
            k % columns
        );
    }
    x += "</Colorset>\n";
    let profiles = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Profiles>\n <Profile name=\"sRGB-elle-V2-srgbtrc.icc\" filename=\"sRGB-elle-V2-srgbtrc.icc\" colorModelId=\"RGBA\" colorDepthId=\"U8\"/>\n</Profiles>\n";
    zip_store(&[("mimetype", b"application/x-krita-palette"), ("colorset.xml", x.as_bytes()), ("profiles.xml", profiles.as_bytes())])
}

