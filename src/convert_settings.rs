//! Réglages de la fenêtre de conversion (conservés entre deux sessions).

use crate::formats::PngLevel;
use crate::imageops::{DitherMethod, Resample, Resize};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum DepthKind {
    Pal8,
    Fallout8,
    Custom8,
    Rgb16,
    Rgb24,
    Rgba32,
}

impl DepthKind {
    pub const ALL: [DepthKind; 6] = [DepthKind::Pal8, DepthKind::Fallout8, DepthKind::Custom8, DepthKind::Rgb16, DepthKind::Rgb24, DepthKind::Rgba32];

    pub fn key(self) -> &'static str {
        match self {
            DepthKind::Pal8 => "c.d8",
            DepthKind::Fallout8 => "c.d8f",
            DepthKind::Custom8 => "c.d8p",
            DepthKind::Rgb16 => "c.d16",
            DepthKind::Rgb24 => "c.d24",
            DepthKind::Rgba32 => "c.d32",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ConvSettings {
    /// Extension du format cible (stable même si la liste des formats évolue).
    pub fmt: String,
    pub depth: DepthKind,
    pub dither: bool,
    pub dither_method: DitherMethod,
    pub resize: Resize,
    pub resample: Resample,
    /// Retire les marges transparentes.
    pub trim: bool,
    pub bg: [u8; 3],
    pub jpeg_quality: u8,
    pub png_level: PngLevel,
    /// KTX2 : BC7 (avec perte) au lieu de RGBA8 sans perte.
    pub ktx2_bc7: bool,
    pub sequence: bool,
    /// Dernier dossier de sortie utilisé.
    pub out_dir: Option<PathBuf>,
    pub pal_path: Option<PathBuf>,
    /// Dernier dossier de sortie effectivement utilisé par une conversion.
    pub last_out_dir: Option<PathBuf>,
    /// Palette personnalisée : créer le fichier .PAL à partir de l'image (sinon choisir un .PAL existant).
    pub create_pal: bool,
    /// Palette créée : forcer l'index 220 à blanc (sous-titres des écrans de mort).
    pub text_white: bool,
}

impl Default for ConvSettings {
    fn default() -> Self {
        ConvSettings {
            fmt: "bmp".to_string(),
            depth: DepthKind::Rgb24,
            dither: false,
            dither_method: DitherMethod::FloydSteinberg,
            resize: Resize::None,
            resample: Resample::Smooth,
            trim: false,
            bg: [0, 0, 0],
            jpeg_quality: 92,
            png_level: PngLevel::Normal,
            ktx2_bc7: false,
            sequence: false,
            out_dir: None,
            pal_path: None,
            last_out_dir: None,
            create_pal: true,
            text_white: false,
        }
    }
}

pub type Outcome = Arc<Mutex<Option<Result<Vec<PathBuf>, String>>>>;

