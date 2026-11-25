//! Settings → Storage: everything KChess keeps on this computer in one place. What it takes in
//! all and what the disk has left, every download with its size and its Download or Delete
//! (the game databases, the puzzles, the speech model, Stockfish), what each account keeps and
//! downloaded, and the local scores and diagnostics.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;

use super::SettingsPage;
use crate::assets::icon;
use crate::core::AppCore;
use crate::log::ResultExt as _;
use crate::pages::Page;
use crate::theme::Palette;
use crate::widgets::{self, DownloadRow, format_bytes};

/// One part of what KChess takes, as the summary's bar and legend show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub label: &'static str,
    pub bytes: u64,
}

/// The parts in the bar's order, from what the page knows of each.
pub fn parts(databases: u64, puzzles: u64, own: u64, speech: u64, engine: u64) -> Vec<Part> {
    vec![
        Part {
            label: "Game databases",
            bytes: databases,
        },
        Part {
            label: "Puzzles",
            bytes: puzzles,
        },
        Part {
            label: "Your games and studies",
            bytes: own,
        },
        Part {
            label: "Speech model and Stockfish",
            bytes: speech.saturating_add(engine),
        },
    ]
}

/// "KChess uses 1.2 GB", the summary's headline.
pub fn headline(parts: &[Part]) -> String {
    let total: u64 = parts.iter().map(|part| part.bytes).sum();
    format!("KChess uses {}", format_bytes(total as f64))
}

impl SettingsPage {
    /// The bytes free on the profile's disk, read again.
    pub(super) fn refresh_free_space(&mut self, cx: &mut Context<Self>) {
        let asked = cx.global::<AppCore>().request(calls::FreeSpace {});
        cx.spawn(async move |this, cx| {
            let free = asked.await.warn_on_err().flatten();
            this.update(cx, |this, cx| {
                this.free_space = free;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What each kind of thing takes now.
    fn storage_parts(&self, cx: &App) -> Vec<Part> {
        let databases = {
            let db = cx.global::<crate::game_db::GameDbHandle>().0.read(cx);
            crate::pages::database::total_bytes(db.databases())
        };
        let puzzles = {
            let db = cx.global::<crate::puzzle_db::PuzzleDbHandle>().0.read(cx);
            if db.installed() {
                db.status.bytes.max(0) as u64
            } else {
                0
            }
        };
        let own = crate::accounts::handle(cx).read(cx).usage.db_bytes;
        let speech = self
            .voice_model
            .as_ref()
            .filter(|status| status.installed)
            .map_or(0, |status| status.bytes);
        let engine = self
            .engine
            .as_ref()
            .filter(|status| status.managed.installed)
            .map_or(0, |status| status.managed.bytes);
        parts(databases, puzzles, own, speech, engine)
    }

    /// What KChess takes in all, what is left on the disk, and a bar of the parts.
    fn storage_summary(&self, cx: &App, palette: &Palette) -> Div {
        let parts = self.storage_parts(cx);
        let total: u64 = parts.iter().map(|part| part.bytes).sum();
        let colors = [
            palette.primary,
            palette.info,
            palette.success,
            palette.warning,
        ];
        let headline = headline(&parts);
        let free = self
            .free_space
            .map(|free| format!("{} free on this disk", format_bytes(free as f64)));
        let shown: Vec<(Part, Hsla)> = parts
            .into_iter()
            .zip(colors)
            .filter(|(part, _)| part.bytes > 0)
            .collect();
        let bar = h_flex()
            .h(px(10.))
            .w_full()
            .rounded_full()
            .overflow_hidden()
            .bg(palette.accented)
            .children(shown.iter().map(|(part, color)| {
                div()
                    .h_full()
                    .w(relative(part.bytes as f32 / total.max(1) as f32))
                    .bg(*color)
            }));
        let legend = h_flex()
            .flex_wrap()
            .gap_x(px(20.))
            .gap_y(px(6.))
            .children(shown.iter().map(|(part, color)| {
                let text = format!("{} {}", part.label, format_bytes(part.bytes as f64));
                h_flex()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .child(div().size(px(8.)).rounded_full().bg(*color))
                    .child(div().text_color(palette.text_muted).child(text))
            }));
        widgets::card(palette)
            .gap(px(12.))
            .child(
                h_flex()
                    .justify_between()
                    .items_end()
                    .flex_wrap()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("storage-total")
                            .test_support()
                            .aria_label(headline.clone())
                            .text_size(px(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(headline),
                    )
                    .children(free.map(|free| {
                        div()
                            .id("storage-free")
                            .test_support()
                            .aria_label(free.clone())
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(free)
                    })),
            )
            .when(total > 0, |card| card.child(bar).child(legend))
    }

    /// The game databases' row: how many and their size, with Manage.
    fn databases_row(&self, cx: &App) -> DownloadRow {
        let db = cx.global::<crate::game_db::GameDbHandle>().0.read(cx);
        let count = db.databases().len();
        let row = |detail: String| {
            DownloadRow::new(
                "storage-databases",
                icon("database"),
                "Game databases",
                detail,
            )
        };
        if count == 0 {
            return row("Master, engine and players' games to search".into()).action(
                Button::new("storage-databases-open")
                    .outline()
                    .small()
                    .icon(icon("download"))
                    .label("Get games")
                    .on_click(|_, window, cx| crate::shell::open(Page::Database, window, cx)),
            );
        }
        let bytes = crate::pages::database::total_bytes(db.databases());
        row(format!(
            "{} · {}",
            widgets::format_counted(count as f64, "database", "databases"),
            format_bytes(bytes as f64)
        ))
        .action(
            Button::new("storage-databases-manage")
                .outline()
                .small()
                .icon(icon("hard-drive"))
                .label("Manage")
                .on_click(|_, window, cx| crate::pages::database::manage_databases(window, cx)),
        )
    }

    /// Stockfish's row, where an official build exists for this computer or one is downloaded.
    fn engine_row(&self, cx: &mut Context<Self>) -> Option<DownloadRow> {
        let status = self.engine.as_ref()?;
        let managed = &status.managed;
        if !status.can_download && !managed.installed {
            return None;
        }
        let busy = self.engine_busy;
        let detail = if busy {
            "Working…".to_string()
        } else if managed.installed {
            let version = managed
                .version
                .as_ref()
                .map(|version| format!("Version {version} · "))
                .unwrap_or_default();
            format!("{version}{}", format_bytes(managed.bytes as f64))
        } else {
            "Official native build".to_string()
        };
        let mut row = DownloadRow::new("storage-engine", icon("cpu"), "Stockfish", detail);
        if busy {
            row = row.progress(None);
        }
        if status.can_download {
            row =
                row.action(
                    Button::new("storage-engine-download")
                        .outline()
                        .small()
                        .icon(icon("download"))
                        .label(if managed.installed {
                            "Update"
                        } else {
                            "Download"
                        })
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.install_engine_with_room(window, cx)
                        })),
                );
        }
        if managed.installed {
            row = row.action(
                Button::new("storage-engine-delete")
                    .ghost()
                    .small()
                    .icon(icon("trash-2"))
                    .accessibility_label("Delete Stockfish")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.ask_delete_engine(window, cx)),
                    ),
            );
        }
        Some(row)
    }

    /// Everything KChess downloads, one row each.
    fn downloads_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let rows: Vec<DownloadRow> = [
            Some(self.databases_row(cx)),
            Some(crate::puzzle_db::row(cx)),
            Some(self.voice_model_row(cx)),
            self.engine_row(cx),
        ]
        .into_iter()
        .flatten()
        .collect();
        widgets::card(palette)
            .gap(px(4.))
            .child(widgets::heading("Downloads", palette))
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .when(index > 0, |line| {
                        line.border_t_1().border_color(palette.border)
                    })
                    .child(row)
            }))
    }

    pub(super) fn storage(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        v_flex()
            .gap(px(16.))
            .child(self.storage_summary(cx, palette))
            .child(self.downloads_card(cx, palette))
            .child(self.accounts_card(cx, palette))
            .child(self.scores_card(cx, palette))
    }
}

#[cfg(test)]
mod tests {
    use super::{headline, parts};

    /// The speech model and Stockfish share a part; the headline is every part's sum.
    #[test]
    fn the_summary_adds_up_every_part() {
        let mb = 1024 * 1024;
        let all = parts(500 * mb, 100 * mb, 20 * mb, 40 * mb, 60 * mb);
        let labels: Vec<_> = all.iter().map(|part| part.label).collect();
        assert_eq!(
            labels,
            [
                "Game databases",
                "Puzzles",
                "Your games and studies",
                "Speech model and Stockfish"
            ]
        );
        assert_eq!(all[3].bytes, 100 * mb);
        assert_eq!(headline(&all), "KChess uses 720 MB");
        assert_eq!(headline(&parts(0, 0, 0, 0, 0)), "KChess uses 0 B");
    }
}
