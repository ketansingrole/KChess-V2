//! Settings → Offline downloads (`OfflineDownloads.vue`): what KChess keeps on this computer to
//! work without a connection — the chess engine, the puzzle database, and the games and studies
//! saved in the profile. (Offline voice input waits for voice input in this app.)

use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Section, SettingsPage, engine::engine_name};
use crate::assets::icon;
use crate::pages::Page;
use crate::theme::Palette;
use crate::widgets::{self, Look};

impl SettingsPage {
    pub(super) fn offline(&self, cx: &App, palette: &Palette) -> Div {
        let status = self.engine.as_ref();
        let ready = status.is_some_and(|status| status.ready);
        let name = engine_name(status);
        let managed = status
            .map(|status| &status.managed)
            .filter(|managed| managed.installed);
        let engine = widgets::card(palette)
            .gap(px(6.))
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .child(widgets::heading("Chess engine", palette))
                    .child(widgets::badge(
                        "Included with KChess",
                        Look::Success,
                        palette,
                    )),
            )
            .when(ready && !name.is_empty(), |card| {
                card.child(widgets::hint(format!("Selected engine: {name}."), palette))
            })
            .children(managed.map(|managed| {
                let version = managed
                    .version
                    .as_ref()
                    .map(|version| format!(" · {version}"))
                    .unwrap_or_default();
                widgets::hint(
                    format!("Optional native engine installed{version}."),
                    palette,
                )
            }))
            .when(status.is_some() && !ready, |card| {
                card.child(widgets::hint(
                    "Selected engine unavailable. Choose another in Chess engine settings.",
                    palette,
                ))
            })
            .child(
                h_flex().mt(px(6.)).child(
                    Button::new("offline-engine")
                        .outline()
                        .icon(icon("cpu"))
                        .label("Chess engine settings")
                        .on_click(|_, window, cx| {
                            crate::shell::open_settings(Section::Engine, window, cx)
                        }),
                ),
            );
        let puzzles = widgets::card(palette)
            .gap(px(12.))
            .child(widgets::heading("Downloaded puzzles", palette))
            .child(crate::puzzle_db::card(true, cx, palette));
        let saved = widgets::card(palette)
            .gap(px(12.))
            .child(widgets::heading("Saved games and studies", palette))
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        Button::new("offline-studies")
                            .outline()
                            .label("Your studies")
                            .on_click(|_, window, cx| {
                                crate::shell::open(Page::Studies, window, cx)
                            }),
                    )
                    .child(
                        Button::new("offline-history")
                            .outline()
                            .label("Game history")
                            .on_click(|_, window, cx| {
                                crate::shell::open(Page::History, window, cx)
                            }),
                    ),
            );
        v_flex()
            .gap(px(16.))
            .child(engine)
            .child(puzzles)
            .child(saved)
    }
}
