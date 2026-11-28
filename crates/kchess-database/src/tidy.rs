//! Tidying a database: removing the copies of its games that came in twice, and merging the
//! spellings of one player's name ("Carlsen, M" and "Carlsen, Magnus") that different sources
//! wrote, so a search for the player finds all their games.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, PlayerName, PlayerNames};
use kchess_contracts::types::{DatabaseInfo, GameDbName, GameDbNameGroup};
use kchess_ui::icon;

use crate::core::AppCore;
use crate::game_db::GameDbHandle;
use crate::loading::{self, Loading};
use crate::theme::Palette;
use crate::widgets::{self, Pill};

/// Counts the copies in `info`'s database, then asks before removing them.
pub fn remove_duplicates(info: DatabaseInfo, window: &mut Window, cx: &mut App) {
    let count = cx.global::<AppCore>().request(calls::GameDbDuplicates {
        database: DatabaseId(info.id.clone()),
    });
    window
        .spawn(cx, async move |cx| {
            let count = count.await;
            cx.update(|window, cx| match count {
                Ok(0) => window.push_notification(Notification::info("No duplicate games"), cx),
                Ok(count) => ask_remove(info, count, window, cx),
                Err(cause) => window.push_notification(
                    Notification::error(loading::failure("database", &cause))
                        .title("Could not look for duplicates"),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}

fn ask_remove(info: DatabaseInfo, count: u64, window: &mut Window, cx: &mut App) {
    let title = format!(
        "Remove {}?",
        widgets::format_counted(count as f64, "duplicate", "duplicates")
    );
    let description = format!(
        "Games with the same players, date, result and moves. The first of each stays in {}.",
        info.name
    );
    widgets::confirm_danger(window, cx, title, description, "Remove", move |_, cx| {
        let db = cx.global::<GameDbHandle>().0.clone();
        db.update(cx, |db, cx| db.remove_duplicates(info.id.clone(), cx));
    });
}

/// Lists the names of `info`'s database that look like one player's, to merge.
pub fn merge_names(info: DatabaseInfo, window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| NamesDialog::new(info, cx));
    widgets::open_dialog(window, cx, move |dialog_box, _, _| {
        dialog_box
            .title("Merge player names")
            .w(px(560.))
            .child(dialog.clone())
    });
}

/// One player's spellings, and the one kept.
struct Group {
    names: Vec<GameDbName>,
    keep: usize,
}

impl Group {
    /// The name kept, and the names it replaces.
    fn merge(&self) -> (String, Vec<String>) {
        let kept = self.names[self.keep].name.clone();
        let gone = self
            .names
            .iter()
            .filter(|name| name.name != kept)
            .map(|name| name.name.clone())
            .collect();
        (kept, gone)
    }
}

pub struct NamesDialog {
    /// The scroll of the list inside it, kept to the list (`widgets::nested_scroll`).
    list_scroll: widgets::NestedScroll,
    database: String,
    groups: Vec<Group>,
    loading: Loading,
    /// A merge runs: nothing else is merged until it ends.
    merging: bool,
    error: Option<SharedString>,
}

impl NamesDialog {
    fn new(info: DatabaseInfo, cx: &mut Context<Self>) -> Self {
        let mut dialog = NamesDialog {
            list_scroll: widgets::NestedScroll::new(),
            database: info.id,
            groups: Vec::new(),
            loading: Loading::default(),
            merging: false,
            error: None,
        };
        let groups = cx.global::<AppCore>().request(calls::GameDbNameGroups {
            database: DatabaseId(dialog.database.clone()),
        });
        dialog.loading.load(
            cx,
            "database",
            |dialog| &mut dialog.loading,
            groups,
            |dialog, answer: Result<Vec<GameDbNameGroup>, _>, _| {
                dialog.groups = answer
                    .unwrap_or_default()
                    .into_iter()
                    .map(|group| Group {
                        names: group.names,
                        keep: 0,
                    })
                    .collect();
            },
        );
        dialog
    }

    /// Merges the groups `which` lists, one after another; each leaves the list once merged.
    fn merge(&mut self, which: Vec<usize>, cx: &mut Context<Self>) {
        if self.merging {
            return;
        }
        let merges: Vec<(String, Vec<String>)> = which
            .into_iter()
            .filter_map(|index| self.groups.get(index))
            .map(Group::merge)
            .filter(|(_, gone)| !gone.is_empty())
            .collect();
        if merges.is_empty() {
            return;
        }
        self.merging = true;
        self.error = None;
        cx.notify();
        let core = cx.global::<AppCore>().clone();
        let database = self.database.clone();
        cx.spawn(async move |this, cx| {
            for (kept, gone) in merges {
                let merged = core
                    .request(calls::GameDbMergeNames {
                        database: DatabaseId(database.clone()),
                        into: PlayerName(kept.clone()),
                        names: PlayerNames(gone),
                    })
                    .await;
                let failed = this
                    .update(cx, |this, cx| {
                        let failed = match merged {
                            Ok(_) => {
                                this.groups
                                    .retain(|group| group.names[group.keep].name != kept);
                                false
                            }
                            Err(cause) => {
                                this.error = Some(loading::failure("database", &cause).into());
                                true
                            }
                        };
                        cx.notify();
                        failed
                    })
                    .unwrap_or(true);
                if failed {
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.merging = false;
                // The page lists the games again under their names.
                let db = cx.global::<GameDbHandle>().0.clone();
                db.update(cx, |db, cx| db.refresh(cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_group(
        &self,
        index: usize,
        group: &Group,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement + use<> {
        let pills = group
            .names
            .iter()
            .map(|name| Pill {
                icon: None,
                label: name.name.clone().into(),
                badge: Some(widgets::format_count(name.games as f64).into()),
                disabled: self.merging,
            })
            .collect();
        let page = cx.entity().downgrade();
        let choose = move |keep: usize, _: &mut Window, cx: &mut App| {
            page.update(cx, |this, cx| {
                if let Some(group) = this.groups.get_mut(index) {
                    group.keep = keep;
                    cx.notify();
                }
            })
            .ok();
        };
        let kept = group.names[group.keep].name.clone();
        h_flex()
            .id(("names-group", index))
            .gap(px(12.))
            .py(px(8.))
            .when(index > 0, |row| {
                row.border_t_1().border_color(palette.border)
            })
            .child(div().flex_1().min_w_0().child(widgets::wrapping_pills(
                "names-keep",
                pills,
                group.keep,
                choose,
                palette,
            )))
            .child(
                Button::new(("names-merge", index))
                    .outline()
                    .small()
                    .label("Merge")
                    .accessibility_label(format!("Merge into {kept}"))
                    .disabled(self.merging)
                    .on_click(cx.listener(move |this, _, _, cx| this.merge(vec![index], cx))),
            )
    }
}

impl Render for NamesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let body = if self.loading.busy() {
            widgets::loading("Finding names…", &palette).into_any_element()
        } else if let Some(error) = self.loading.error() {
            div()
                .text_color(palette.error)
                .child(error.clone())
                .into_any_element()
        } else if self.groups.is_empty() {
            widgets::empty_state(icon("users"), "No names to merge", None, &palette)
                .py(px(20.))
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = self
                .groups
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    self.render_group(index, group, cx, &palette)
                        .into_any_element()
                })
                .collect();
            v_flex()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child("The chosen name replaces the others in every game."),
                )
                .child(
                    widgets::nested_scroll(
                        v_flex().id("names-groups").max_h(px(420.)),
                        &self.list_scroll,
                    )
                    .children(rows),
                )
                .into_any_element()
        };
        let all = self.groups.len();
        v_flex()
            .gap(px(12.))
            .child(body)
            .child(widgets::dialog_footer(
                self.error.clone().map(|error| ("names-error", error)),
                None,
                [
                    Button::new("names-done")
                        .ghost()
                        .label("Done")
                        .on_click(|_, window, cx| window.close_dialog(cx))
                        .into_any_element(),
                    Button::new("names-merge-all")
                        .primary()
                        .icon(icon("users"))
                        .label("Merge all")
                        .loading(self.merging)
                        .disabled(self.merging || all < 2)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.merge((0..all).collect(), cx)),
                        )
                        .into_any_element(),
                ],
                &palette,
            ))
    }
}
