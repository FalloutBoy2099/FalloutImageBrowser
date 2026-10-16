//! FalloutImageBrowser : visionneuse et convertisseur d'images (FRM, PAL, RIX, ART, ...).

// Traitement de pixels : les boucles par indice et les types de retour détaillés y sont voulus, et plus lisibles que
// leurs équivalents en itérateurs. Les autres avertissements de clippy sont bloquants dans la CI.
#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments, clippy::enum_variant_names, clippy::manual_checked_ops)]

pub mod app;
pub mod assoc;
pub mod batch;
pub mod batch_ui;
pub mod cli;
pub mod convert;
pub mod convert_ui;
pub mod dat;
pub mod dialog;
pub mod fileops;
pub mod formats;
pub mod frm_ui;
pub mod hash;
pub mod i18n;
pub mod icon;
pub mod icons;
pub mod imageops;
pub mod loader;
pub mod model;
pub mod opener;
pub mod palette;
pub mod palette_export;
pub mod palette_ui;
pub mod props;
pub mod tree;
pub mod ui;
pub mod view;
pub mod winprefs;
pub mod zipread;
