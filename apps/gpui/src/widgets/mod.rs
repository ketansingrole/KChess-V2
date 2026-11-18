//! The app's own widgets: cards and search highlights; and, from `kchess_board::widgets`, the
//! board pages' widgets and the generic ones every page draws with.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
pub use kchess_board::widgets::*;

use crate::assets::icon;
use crate::theme::{Palette, radii};

mod cards;
mod highlight;

pub use cards::*;
pub use highlight::*;
