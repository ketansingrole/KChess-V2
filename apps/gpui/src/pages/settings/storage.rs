//! Settings → Storage: everything KChess keeps on this computer in one place. One list of what it
//! takes, the largest first, each with what can be done about it: the app itself and its service,
//! every download (the game databases, the puzzles, the speech model, Stockfish, the endgame
//! tablebases) with its Update or Delete, the files the Electron app left with Clean up, and
//! what else the profile holds, so nothing is left out. Then what can still be downloaded, under
//! what the disk has left, what each account keeps and downloaded, and the local scores and
//! diagnostics. A bar over the list draws each row in the row's own colour, and the list is
//! its legend. Nothing beside the bar speaks of the disk: with the free space at its end and a
//! part in a gray near its track's, people read it as their whole disk.

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
    /// What the Electron app left in the profile.
    pub leftovers: u64,
}

impl Measured {
    /// What has a row of its own in the profile.
    fn named(&self) -> u64 {
        [
            self.databases,
            self.puzzles,
            self.own,
            self.speech,
            self.engine,
            self.tablebases,
            self.leftovers,
        ]
        .into_iter()
        .fold(0u64, u64::saturating_add)
    }

    /// What the profile holds besides what has a row (logs, themes, the databases' working
    /// files): never less than nothing, and nothing until the profile is measured.
    pub fn other(&self) -> u64 {
        self.profile
            .map_or(0, |profile| profile.saturating_sub(self.named()))
    }

    /// All KChess takes: the app and every row, so the rows add up to it.
    pub fn total(&self) -> u64 {
        self.app
            .unwrap_or(0)
            .saturating_add(self.named())
            .saturating_add(self.other())
    }
}

/// "KChess uses 1.2 GB", the list's headline.
pub fn headline(total: u64) -> String {
    format!("KChess uses {}", format_bytes(total as f64))
}

/// The rows by what they take, the largest first; rows of one size keep their order.
pub fn largest_first<T>(mut rows: Vec<(u64, T)>) -> Vec<(u64, T)> {
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    rows
}

/// What a row on this computer is. Each keeps its colour in the bar and on its icon wherever its
/// size puts it in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App,
    Leftovers,
    Own,
    Engine,
    Puzzles,
    Databases,
    Tablebases,
    Speech,
    Other,
}

impl Kind {
    /// Its row's title.
    pub fn title(self) -> &'static str {
        match self {
            Kind::App => "KChess app",
            Kind::Leftovers => "Files from earlier versions",
            Kind::Own => "Your games and studies",
            Kind::Engine => "Stockfish",
            Kind::Puzzles => "Puzzles",
            Kind::Databases => "Game databases",
            Kind::Tablebases => "Endgame tablebases",
            Kind::Speech => "Voice input",
            Kind::Other => "Logs and other files",
        }
    }

    /// Its colour: one of the categorical palette's, and for the profile's other files the
    /// toned gray, which stands out from the bar's track where a gray near the track's did not.
    pub fn color(self, palette: &Palette) -> Hsla {
        let slot = match self {
            Kind::App => 1,
            Kind::Leftovers => 2,
            Kind::Own => 3,
            Kind::Engine => 4,
            Kind::Puzzles => 5,
            Kind::Databases => 6,
            Kind::Tablebases => 7,
            Kind::Speech => 8,
            Kind::Other => return palette.toned,
        };
        kchess_ui::theme::categorical(slot, palette.dark)
    }
}

impl SettingsPage {
    /// The bytes free on the profile's disk, what the profile holds, what the Electron app left
    /// in it, and what the app and its service take, read again.
    pub(super) fn refresh_free_space(&mut self, cx: &mut Context<Self>) {
        let free = cx.global::<AppCore>().request(calls::FreeSpace {});
        let profile = cx.global::<AppCore>().request(calls::ProfileSize {});
        let leftovers = cx.global::<AppCore>().request(calls::LeftoverSize {});
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
            let leftovers = leftovers.await.warn_on_err();
            let app = app.await;
            this.update(cx, |this, cx| {
                this.free_space = free;
                this.profile_bytes = profile;
                this.leftover_bytes = leftovers;
                this.app_bytes = app;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What each kind of thing takes now.
    fn measured(&self, cx: &App) -> Measured {
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
        Measured {
            app: self.app_bytes,
            profile: self.profile_bytes,
            databases,
            puzzles,
            own,
            speech,
            engine,
            tablebases: self.tablebase_bytes(),
            leftovers: self.leftover_bytes.unwrap_or(0),
        }
    }

    /// The files the Electron app left: what they take and Clean up, which asks first.
    fn leftovers_row(&self, bytes: u64, cx: &mut Context<Self>) -> DownloadRow {
        let busy = self.leftovers_busy;
        DownloadRow::new(
            "storage-leftovers",
            icon("history"),
            Kind::Leftovers.title(),
            format_bytes(bytes as f64),
        )
        .error("storage-leftovers-error", self.leftovers_error.clone())
        .action(
            Button::new("storage-leftovers-remove")
                .outline()
                .small()
                .icon(icon("trash-2"))
                .label("Clean up")
                .accessibility_label("Clean up files from earlier versions")
                .loading(busy)
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.ask_remove_leftovers(window, cx))),
        )
    }

    fn ask_remove_leftovers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        widgets::confirm_danger(
            window,
            cx,
            "Clean up files from earlier versions?",
            "KChess no longer uses them.",
            "Clean up",
            move |_, cx| {
                this.update(cx, |this, cx| this.remove_leftovers(cx)).ok();
            },
        );
    }

    fn remove_leftovers(&mut self, cx: &mut Context<Self>) {
        self.leftovers_busy = true;
        self.leftovers_error = None;
        cx.notify();
        let remove = cx.global::<AppCore>().request(calls::RemoveLeftovers {});
        cx.spawn(async move |this, cx| {
            let removed = remove.await;
            this.update(cx, |this, cx| {
                this.leftovers_busy = false;
                if let Err(cause) = removed {
                    this.leftovers_error =
                        Some(kchess_app::loading::failure("leftovers", &cause).into());
                }
                this.refresh_free_space(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A row with nothing to do but read what it takes.
    fn size_row(id: &'static str, glyph: &'static str, kind: Kind, bytes: u64) -> DownloadRow {
        DownloadRow::new(id, icon(glyph), kind.title(), format_bytes(bytes as f64))
    }

    /// Everything KChess keeps on this computer, the largest first, under what it takes in all;
    /// then what it can still download, under what the disk has left. A download stays with
    /// those to download until it is done, so a row never moves while it runs.
    fn storage_lists(&self, cx: &mut Context<Self>, palette: &Palette) -> (Div, Option<Div>) {
        let measured = self.measured(cx);
        let mut kept: Vec<(u64, (Kind, DownloadRow))> = Vec::new();
        let mut offered: Vec<DownloadRow> = Vec::new();
        if let Some(app) = measured.app {
            kept.push((
                app,
                (
                    Kind::App,
                    Self::size_row("storage-app", "monitor", Kind::App, app),
                ),
            ));
        }
        let databases = cx
            .global::<crate::game_db::GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .len();
        if databases > 0 {
            kept.push((
                measured.databases,
                (Kind::Databases, self.databases_row(cx)),
            ));
        } else {
            offered.push(self.databases_row(cx));
        }
        let puzzles = crate::puzzle_db::row(cx);
        if measured.puzzles > 0 {
            kept.push((measured.puzzles, (Kind::Puzzles, puzzles)));
        } else {
            offered.push(puzzles);
        }
        let voice = self.voice_model_row(cx);
        if measured.speech > 0 {
            kept.push((measured.speech, (Kind::Speech, voice)));
        } else {
            offered.push(voice);
        }
        if let Some(engine) = self.engine_row(cx) {
            if measured.engine > 0 {
                kept.push((measured.engine, (Kind::Engine, engine)));
            } else {
                offered.push(engine);
            }
        }
        let tablebases = self.tablebase_row(cx);
        let downloading = self.tablebase_progress.is_some()
            || self.tablebases.as_ref().is_some_and(|status| status.busy);
        if measured.tablebases > 0 && !downloading {
            kept.push((measured.tablebases, (Kind::Tablebases, tablebases)));
        } else {
            offered.push(tablebases);
        }
        if measured.own > 0 {
            kept.push((
                measured.own,
                (
                    Kind::Own,
                    Self::size_row("storage-own", "library-big", Kind::Own, measured.own),
                ),
            ));
        }
        if measured.leftovers > 0 || self.leftovers_error.is_some() {
            kept.push((
                measured.leftovers,
                (Kind::Leftovers, self.leftovers_row(measured.leftovers, cx)),
            ));
        }
        let other = measured.other();
        if other > 0 {
            kept.push((
                other,
                (
                    Kind::Other,
                    Self::size_row("storage-other", "file-text", Kind::Other, other),
                ),
            ));
        }
        let headline = headline(measured.total());
        let kept = largest_first(kept);
        // The rows fill the bar between them, in the list's order.
        let filled = kept
            .iter()
            .fold(0u64, |total, (bytes, _)| total.saturating_add(*bytes))
            .max(1);
        let bar = h_flex()
            .h(px(10.))
            .w_full()
            .rounded_full()
            .overflow_hidden()
            .bg(palette.accented)
            .children(
                kept.iter()
                    .enumerate()
                    .filter(|(_, (bytes, _))| *bytes > 0)
                    .map(|(index, (bytes, (kind, _)))| {
                        div()
                            .id(("storage-segment", index))
                            .test_support()
                            .aria_label(kind.title())
                            .h_full()
                            .w(relative(*bytes as f32 / filled as f32))
                            .bg(kind.color(palette))
                    }),
            );
        let rows = kept
            .into_iter()
            .map(|(_, (kind, row))| row.tint(kind.color(palette)))
            .collect();
        let list = widgets::card(palette)
            .gap(px(4.))
            .child(
                div()
                    .id("storage-total")
                    .test_support()
                    .aria_label(headline.clone())
                    .pb(px(4.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(headline),
            )
            .child(div().pb(px(4.)).child(bar))
            .children(separated(rows, palette));
        let free = self
            .free_space
            .map(|free| format!("{} free on this disk", format_bytes(free as f64)));
        let downloads = (!offered.is_empty()).then(|| {
            widgets::card(palette)
                .gap(px(4.))
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(widgets::heading("Downloads", palette))
                        .children(free.map(|free| {
                            div()
                                .id("storage-free")
                                .test_support()
                                .aria_label(free.clone())
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(free)
                        })),
                )
                .children(separated(offered, palette))
        });
        (list, downloads)
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
            return row("Broadcast, engine and players' games to search".into()).action(
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

    pub(super) fn storage(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let (list, downloads) = self.storage_lists(cx, palette);
        v_flex()
            .gap(px(16.))
            .child(list)
            .children(downloads)
            .child(self.accounts_card(cx, palette))
            .child(self.scores_card(cx, palette))
    }
}

/// Rows one under another, a line between each.
fn separated(rows: Vec<DownloadRow>, palette: &Palette) -> impl Iterator<Item = Div> + use<> {
    let border = palette.border;
    rows.into_iter().enumerate().map(move |(index, row)| {
        div()
            .when(index > 0, |line| line.border_t_1().border_color(border))
            .child(row)
    })
}

#[cfg(test)]
mod tests {
    use super::{Kind, Measured, headline, largest_first};
    use gpui_kit::Hsla;
    use kchess_domain::theme::Rgb;
    use kchess_ui::theme::{Palette, colors};

    const KINDS: [Kind; 9] = [
        Kind::App,
        Kind::Leftovers,
        Kind::Own,
        Kind::Engine,
        Kind::Puzzles,
        Kind::Databases,
        Kind::Tablebases,
        Kind::Speech,
        Kind::Other,
    ];

    /// Every theme's palette, light and dark.
    fn palettes() -> Vec<(String, Palette)> {
        let presets: Vec<kchess_contracts::types::AppTheme> =
            serde_json::from_value(kchess_domain::theme::presets().clone()).unwrap();
        std::iter::once("kchess".to_string())
            .chain(presets.into_iter().map(|theme| theme.id))
            .flat_map(|id| {
                [false, true].map(|dark| {
                    let palette = Palette::of(&colors(&id, &[], dark), dark);
                    (format!("{id}, dark {dark}"), palette)
                })
            })
            .collect()
    }

    fn rgb(color: Hsla) -> Rgb {
        let color = color.to_rgb();
        Rgb(
            f64::from(color.r) * 255.,
            f64::from(color.g) * 255.,
            f64::from(color.b) * 255.,
        )
    }

    /// The app, every row and what the profile holds besides them add up to all KChess takes.
    #[test]
    fn the_rows_add_up_to_everything_kchess_takes() {
        let mb = 1024 * 1024;
        let measured = Measured {
            app: Some(200 * mb),
            profile: Some(830 * mb),
            databases: 500 * mb,
            puzzles: 100 * mb,
            own: 20 * mb,
            speech: 40 * mb,
            engine: 50 * mb,
            tablebases: 10 * mb,
            leftovers: 100 * mb,
        };
        assert_eq!(measured.other(), 10 * mb, "the profile's other files");
        assert_eq!(headline(measured.total()), "KChess uses 1.0 GB");
        // Not yet measured: only what is known, and never less than nothing.
        let early = Measured {
            profile: Some(mb),
            databases: 5 * mb,
            ..Measured::default()
        };
        assert_eq!((early.other(), early.total()), (0, 5 * mb));
        assert_eq!(headline(Measured::default().total()), "KChess uses 0 B");
    }

    /// The largest first; rows of one size keep their order.
    #[test]
    fn rows_are_listed_largest_first() {
        let rows = vec![(10, "logs"), (330, "app"), (40, "stockfish"), (10, "own")];
        let order: Vec<_> = largest_first(rows)
            .into_iter()
            .map(|(_, row)| row)
            .collect();
        assert_eq!(order, ["app", "stockfish", "logs", "own"]);
    }

    /// No two rows share a colour in any theme, so each part of the bar names one row.
    #[test]
    fn every_row_has_its_own_colour() {
        for (theme, palette) in palettes() {
            for (index, kind) in KINDS.iter().enumerate() {
                for other in &KINDS[index + 1..] {
                    assert_ne!(
                        kind.color(&palette),
                        other.color(&palette),
                        "{theme}: {kind:?}, {other:?}"
                    );
                }
            }
        }
    }

    /// The one gray part stands out from the bar's track by WCAG's 3:1 for non-text contrast in
    /// every theme. The profile's other files were drawn in a gray at 1.25 to 2.91:1 (1.45 in
    /// the dark theme): the bar seemed to stop short, and people read the rest as their disk's
    /// free space.
    #[test]
    fn the_gray_part_stands_out_from_the_track() {
        for (theme, palette) in palettes() {
            let contrast = rgb(Kind::Other.color(&palette)).contrast(rgb(palette.accented));
            assert!(contrast >= 3., "{theme}: {contrast:.2}:1");
        }
    }
}
