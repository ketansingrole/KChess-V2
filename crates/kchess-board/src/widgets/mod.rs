//! The board pages' own widgets: the player line above and below a board, the move list and the
//! board's setting rows; and, from `kchess_ui::widgets`, the generic widgets every page draws with.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_ui::icon;
pub use kchess_ui::widgets::*;

use crate::theme::{Palette, radii, tint};

mod moves;
mod players;
mod setting_rows;
#[cfg(test)]
mod tests;

pub use moves::*;
pub use players::*;
pub use setting_rows::*;
