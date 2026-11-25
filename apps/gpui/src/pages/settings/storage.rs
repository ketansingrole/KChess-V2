//! Settings → Storage: everything KChess keeps on this computer in one place. What it takes in
//! all (the app itself, its service and every file in the profile, so nothing is left out) and
//! what the disk has left, every download with its size and its Download or Delete
//! (the game databases, the puzzles, the speech model, Stockfish, the endgame tablebases), what
//! each account keeps and downloaded, and the local scores and diagnostics.

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

/// What the page has measured, each in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measured {
    /// The installed app and the copy of its service it runs from; unknown in a development run
    /// outside a bundle.
    pub app: Option<u64>,
    /// Every file in the profile's folder, which holds all the rest.
    pub profile: Option<u64>,
    pub databases: u64,
    pub puzzles: u64,
    pub own: u64,
    pub speech: u64,
    pub engine: u64,
    pub tablebases: u64,
}

/// The parts in the bar's order. What the profile holds besides the named parts (logs, themes,
/// the databases' working files) is its own part, so the parts add up to all KChess takes.
pub fn parts(measured: &Measured) -> Vec<Part> {
    let named = [
        ("Game databases", measured.databases),
        ("Puzzles", measured.puzzles),
        ("Your games and studies", measured.own),
        (
            "Stockfish, speech and tablebases",
            measured
                .speech
                .saturating_add(measured.engine)
                .saturating_add(measured.tablebases),
        ),
    ];
    let named_total = named
        .iter()
        .fold(0u64, |total, (_, bytes)| total.saturating_add(*bytes));
    let other = measured
        .profile
        .map_or(0, |profile| profile.saturating_sub(named_total));
    std::iter::once(("KChess app", measured.app.unwrap_or(0)))
        .chain(named)
        .chain(std::iter::once(("Logs and other files", other)))
        .map(|(label, bytes)| Part { label, bytes })
        .collect()
}

/// "KChess uses 1.2 GB", the summary's headline.
pub fn headline(parts: &[Part]) -> String {
    let total: u64 = parts.iter().map(|part| part.bytes).sum();
    format!("KChess uses {}", format_bytes(total as f64))
}

impl SettingsPage {
    /// The bytes free on the profile's disk, what the profile holds, and what the app and its
    /// service take, read again.
    pub(super) fn refresh_free_space(&mut self, cx: &mut Context<Self>) {
        let free = cx.global::<AppCore>().request(calls::FreeSpace {});
        let profile = cx.global::<AppCore>().request(calls::ProfileSize {});
        let app = cx.background_spawn(async move {
            let install = crate::updates::install_path()?;
            let service = crate::core::service_folder();
            Some(kchess_host::disk::folder_bytes(&install).saturating_add(
                service.map_or(0, |folder| kchess_host::disk::folder_bytes(&folder)),
            ))
        });
        cx.spawn(async move |this, cx| {
            let free = free.await.warn_on_err().flatten();
            let profile = profile.await.warn_on_err();
            let app = app.await;
            this.update(cx, |this, cx| {
                this.free_space = free;
                this.profile_bytes = profile;
                this.app_bytes = app;
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
        parts(&Measured {
            app: self.app_bytes,
            profile: self.profile_bytes,
            databases,
            puzzles,
            own,
            speech,
            engine,
            tablebases: self.tablebase_bytes(),
        })
    }

    /// What KChess takes in all, what is left on the disk, and a bar of the parts.
    fn storage_summary(&self, cx: &App, palette: &Palette) -> Div {
        let parts = self.storage_parts(cx);
        let total: u64 = parts.iter().map(|part| part.bytes).sum();
        let colors = [
            palette.dimmed,
            palette.primary,
            palette.info,
            palette.success,
            palette.warning,
            palette.border_accented,
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
        let legend = h_flex().flex_wrap().gap_x(px(20.)).gap_y(px(6.)).children(
            shown.iter().enumerate().map(|(index, (part, color))| {
                let text = format!("{} {}", part.label, format_bytes(part.bytes as f64));
                h_flex()
                    .id(("storage-part", index))
                    .test_support()
                    .aria_label(text.clone())
                    .gap(px(6.))
                    .text_size(px(12.))
                    .child(div().size(px(8.)).rounded_full().bg(*color))
                    .child(div().text_color(palette.text_muted).child(text))
            }),
        );
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
            Some(self.tablebase_row(cx)),
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
    use super::{Measured, headline, parts};

    /// The app and what the profile holds besides the named parts are parts too, so the
    /// headline is all KChess takes.
    #[test]
    fn the_summary_adds_up_everything_kchess_takes() {
        let mb = 1024 * 1024;
        let measured = Measured {
            app: Some(200 * mb),
            profile: Some(730 * mb),
            databases: 500 * mb,
            puzzles: 100 * mb,
            own: 20 * mb,
            speech: 40 * mb,
            engine: 50 * mb,
            tablebases: 10 * mb,
        };
        let all = parts(&measured);
        let labels: Vec<_> = all.iter().map(|part| part.label).collect();
        assert_eq!(
            labels,
            [
                "KChess app",
                "Game databases",
                "Puzzles",
                "Your games and studies",
                "Stockfish, speech and tablebases",
                "Logs and other files"
            ]
        );
        assert_eq!(all[4].bytes, 100 * mb);
        assert_eq!(all[5].bytes, 10 * mb, "the profile's other files");
        assert_eq!(headline(&all), "KChess uses 930 MB");
        // Not yet measured: only what is known, and never less than nothing.
        let early = parts(&Measured {
            profile: Some(mb),
            databases: 5 * mb,
            ..Measured::default()
        });
        assert_eq!((early[0].bytes, early[5].bytes), (0, 0));
        assert_eq!(headline(&parts(&Measured::default())), "KChess uses 0 B");
    }
}
