//! Pieces the board pages share: the player line above and below a board, the move list, cards
//! and setting rows; and, from `kchess_ui::widgets`, the generic widgets every page draws with.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
pub use kchess_ui::widgets::*;

use crate::assets::icon;
use crate::theme::{Palette, radii, tint};

mod cards;
mod highlight;
mod moves;
mod players;
mod setting_rows;
#[cfg(test)]
mod tests;

pub use cards::*;
pub use highlight::*;
pub use moves::*;
pub use players::*;
pub use setting_rows::*;
