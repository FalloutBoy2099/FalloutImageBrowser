//! Interface : menus, barre d'outils, arborescence, propriétés et zone d'affichage.

mod actions;
mod bg_picker;
mod canvas;
mod dialogs;
mod input;
mod menu;
mod props_panel;
mod toolbar;
mod tree_panel;
mod widgets;

pub use widgets::extra_window;

const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 64.0;
