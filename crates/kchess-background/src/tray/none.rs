//! A system without a tray KChess knows: nothing shows, so closing the window quits.

use super::{Events, Icon, Item};

pub struct Tray;

impl Tray {
    pub fn new(_: &str, _: &Icon, _: Vec<Item>, _: Events) -> Result<Tray, String> {
        Err("This system has no tray KChess can use.".into())
    }

    pub fn set_menu(&mut self, _: Vec<Item>) {}
}
