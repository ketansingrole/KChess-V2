//! One line above the analysis board: which study the board
//! is, saved as you go, with its chapters; or a way to keep the board as one. The library itself is
//! the Studies page.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::Page;
use super::analysis::AnalysisPage;
use crate::assets::icon;
use crate::name_dialog::NameDialog;
use crate::studies::{MAX_CHAPTERS, StudyHandle, chapter_items, now, time_ago};
use crate::theme::Palette;
use crate::widgets::Choice;

/// The bar's height and its gap to the board, which the board's size leaves room for.
pub const STUDY_BAR: Pixels = px(48.);
/// "Saved 2 min ago" is redrawn this often.
const CLOCK: Duration = Duration::from_secs(30);
const RESULTS: [&str; 4] = ["*", "1-0", "0-1", "1/2-1/2"];

pub struct StudyBar {
    analysis: WeakEntity<AnalysisPage>,
    name: Entity<InputState>,
    chapters: Entity<SelectState<Vec<Choice>>>,
    chapter_error: Option<String>,
    /// The study, chapter, name and chapter list the fields were last filled from.
    shown: (String, String, String, Vec<(String, String)>),
    _clock: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl StudyBar {
    pub fn new(
        analysis: WeakEntity<AnalysisPage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> StudyBar {
        let name = cx.new(|cx| InputState::new(window, cx));
        let chapters = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let library = cx.global::<StudyHandle>().0.clone();
        let subscriptions = vec![
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.rename(window, cx);
                }
            }),
            cx.subscribe(
                &chapters,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.select_chapter(value.to_string(), cx);
                    }
                },
            ),
            cx.observe(&library, |_, _, cx| cx.notify()),
        ];
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        StudyBar {
            analysis,
            name,
            chapters,
            chapter_error: None,
            shown: Default::default(),
            _clock: clock,
            _subscriptions: subscriptions,
        }
    }

    /// The open study, and its chapter id.
    fn open(&self, cx: &App) -> Option<(Value, String)> {
        let analysis = self.analysis.upgrade()?;
        let analysis = analysis.read(cx);
        let study = cx
            .global::<StudyHandle>()
            .0
            .read(cx)
            .study(analysis.study_id())?
            .clone();
        Some((study, analysis.chapter_id().to_string()))
    }

    /// Refill the name and the chapters when another study, chapter or name is shown.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((study, chapter)) = self.open(cx) else {
            return;
        };
        let id = study["id"].as_str().unwrap_or_default().to_string();
        let name = study["name"].as_str().unwrap_or_default().to_string();
        let items = chapter_items(&study);
        let shown = (id, chapter.clone(), name.clone(), items.clone());
        if shown == self.shown {
            return;
        }
        if shown.0 != self.shown.0 || shown.1 != self.shown.1 {
            self.chapter_error = None;
        }
        self.shown = shown;
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        let choices: Vec<Choice> = items
            .into_iter()
            .map(|(value, label)| Choice::new(value, label))
            .collect();
        self.chapters.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_value(&SharedString::from(chapter), window, cx);
        });
    }

    fn library(cx: &App) -> Entity<crate::studies::StudyLibrary> {
        cx.global::<StudyHandle>().0.clone()
    }

    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((study, _)) = self.open(cx) else {
            return;
        };
        let current = study["name"].as_str().unwrap_or_default().to_string();
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() || name == current {
            self.name
                .update(cx, |input, cx| input.set_value(current, window, cx));
            return;
        }
        let library = Self::library(cx);
        let answer = library
            .read(cx)
            .command(json!({ "op": "rename", "id": study["id"], "name": name }));
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(result) => library.update(cx, |library, cx| {
                    library.apply(&result);
                    cx.notify();
                }),
                Err(cause) => {
                    this.name
                        .update(cx, |input, cx| input.set_value(current, window, cx));
                    window.push_notification(
                        Notification::error(cause).title("Could not rename the study"),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn select_chapter(&mut self, chapter: String, cx: &mut Context<Self>) {
        let Some(analysis) = self.analysis.upgrade() else {
            return;
        };
        let study = analysis.read(cx).study_id().to_string();
        if analysis.read(cx).chapter_id() == chapter {
            return;
        }
        self.chapter_error = analysis
            .update(cx, |analysis, cx| {
                analysis.open_study(&study, Some(&chapter), cx)
            })
            .err();
        cx.notify();
    }

    /// Run a chapter command once the board's edits are saved, then show the chapter it names.
    fn chapter_command(
        &mut self,
        command: Value,
        dialog: Option<Entity<NameDialog>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(analysis) = self.analysis.upgrade() else {
            return;
        };
        let study = analysis.read(cx).study_id().to_string();
        let flushed = analysis.update(cx, |analysis, cx| analysis.flush_study(cx));
        let library = Self::library(cx);
        cx.spawn_in(window, async move |this, cx| {
            let outcome = async {
                flushed.await?;
                let answer = this
                    .update(cx, |_, cx| library.read(cx).command(command))
                    .map_err(|_| "The analysis board closed".to_string())?;
                answer.await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                Ok(result) => {
                    library.update(cx, |library, cx| {
                        library.apply(&result);
                        cx.notify();
                    });
                    if let Some(id) = result["id"].as_str() {
                        this.chapter_error = analysis
                            .update(cx, |analysis, cx| analysis.open_study(&study, Some(id), cx))
                            .err();
                    }
                    if dialog.is_some() {
                        window.close_dialog(cx);
                    }
                    cx.notify();
                }
                Err(cause) => match &dialog {
                    Some(dialog) => dialog.update(cx, |dialog, cx| dialog.fail(cause, cx)),
                    None => {
                        this.chapter_error = Some(cause);
                        cx.notify();
                    }
                },
            });
        })
        .detach();
    }

    fn add_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bar = cx.entity().downgrade();
        let Some((study, _)) = self.open(cx) else {
            return;
        };
        NameDialog::open(
            "Add chapter",
            "",
            "Add",
            window,
            cx,
            move |name, dialog, window, cx| {
                let command = json!({ "op": "addChapter", "id": study["id"], "name": name });
                let _ = bar.update(cx, |bar, cx| {
                    bar.chapter_command(command, Some(dialog), window, cx)
                });
            },
        );
    }

    fn rename_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((study, chapter)) = self.open(cx) else {
            return;
        };
        let current = study["chapters"]
            .as_array()
            .and_then(|chapters| chapters.iter().find(|entry| entry["id"] == json!(chapter)))
            .and_then(|entry| entry["name"].as_str())
            .unwrap_or_default()
            .to_string();
        let library = Self::library(cx);
        NameDialog::open(
            "Rename chapter",
            &current,
            "Save",
            window,
            cx,
            move |name, dialog, window, cx| {
                let answer = library.read(cx).command(json!({
                    "op": "renameChapter",
                    "id": study["id"],
                    "chapterId": chapter,
                    "name": name,
                }));
                let library = library.clone();
                window
                    .spawn(cx, async move |cx| {
                        let result = answer.await;
                        let _ = cx.update(|window, cx| match result {
                            Ok(result) => {
                                library.update(cx, |library, cx| {
                                    library.apply(&result);
                                    cx.notify();
                                });
                                window.close_dialog(cx);
                            }
                            Err(cause) => dialog.update(cx, |dialog, cx| dialog.fail(cause, cx)),
                        });
                    })
                    .detach();
            },
        );
    }

    fn duplicate_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((study, chapter)) = self.open(cx) else {
            return;
        };
        let command = json!({ "op": "duplicateChapter", "id": study["id"], "chapterId": chapter });
        self.chapter_command(command, None, window, cx);
    }

    /// Delete the chapter shown: the board lets go of it first, so a pending save cannot bring it
    /// back; the next chapter opens, or this one again when it could not be deleted.
    fn remove_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(analysis) = self.analysis.upgrade() else {
            return;
        };
        let Some((study, chapter)) = self.open(cx) else {
            return;
        };
        let id = study["id"].as_str().unwrap_or_default().to_string();
        let closed = analysis.update(cx, |analysis, cx| analysis.close_study(cx));
        let library = Self::library(cx);
        cx.spawn_in(window, async move |this, cx| {
            if !closed.await {
                let _ = this.update(cx, |this, cx| {
                    this.chapter_error = analysis.read(cx).study_error().map(str::to_string);
                    cx.notify();
                });
                return;
            }
            let Ok(answer) = this.update(cx, |_, cx| {
                library.read(cx).command(json!({
                    "op": "removeChapter",
                    "id": id,
                    "chapterId": chapter,
                }))
            }) else {
                return;
            };
            let result = answer.await;
            let _ = this.update(cx, |this, cx| {
                let next = match &result {
                    Ok(result) => {
                        library.update(cx, |library, cx| {
                            library.apply(result);
                            cx.notify();
                        });
                        result["id"].as_str().map(str::to_string)
                    }
                    Err(cause) => {
                        this.chapter_error = Some(cause.clone());
                        Some(chapter.clone())
                    }
                };
                let opened = analysis.update(cx, |analysis, cx| {
                    analysis.open_study(&id, next.as_deref(), cx)
                });
                if let Err(cause) = opened {
                    this.chapter_error = Some(cause);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_remove_chapter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((study, chapter)) = self.open(cx) else {
            return;
        };
        let name = study["chapters"]
            .as_array()
            .and_then(|chapters| chapters.iter().find(|entry| entry["id"] == json!(chapter)))
            .and_then(|entry| entry["name"].as_str())
            .unwrap_or_default()
            .to_string();
        let cloud = !study["cloud"].is_null();
        let bar = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let bar = bar.clone();
            dialog
                .title(format!("Delete “{name}”?"))
                .description(if cloud {
                    "Removes this chapter from the offline study. The Lichess copy is unchanged."
                } else {
                    "Removes this chapter and all of its moves, comments, and variations"
                })
                .ok_text("Delete chapter")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let _ = bar.update(cx, |bar, cx| bar.remove_chapter(window, cx));
                    true
                })
        });
    }

    fn save_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(analysis) = self.analysis.upgrade() else {
            return;
        };
        let name = analysis
            .read(cx)
            .players()
            .unwrap_or_else(|| Self::library(cx).read(cx).fresh_name("Untitled study"));
        NameDialog::open(
            "Save as study",
            &name,
            "Save study",
            window,
            cx,
            move |name, dialog, window, cx| {
                let saved = analysis.update(cx, |analysis, cx| analysis.save_as_study(name, cx));
                window
                    .spawn(cx, async move |cx| {
                        let saved = saved.await;
                        let _ = cx.update(|window, cx| match saved {
                            Ok(_) => {
                                window.close_dialog(cx);
                                window.push_notification(
                                    Notification::success(
                                        "Changes on the board are now saved to it automatically",
                                    )
                                    .title("Study saved"),
                                    cx,
                                );
                            }
                            Err(cause) => dialog.update(cx, |dialog, cx| dialog.fail(cause, cx)),
                        });
                    })
                    .detach();
            },
        );
    }

    fn save_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(analysis), Some((study, _))) = (self.analysis.upgrade(), self.open(cx)) else {
            return;
        };
        let name: String = study["name"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(113)
            .collect();
        let copy = format!("{name} (copy)");
        let saved = analysis.update(cx, |analysis, cx| analysis.save_as_study(copy.clone(), cx));
        window
            .spawn(cx, async move |cx| {
                let saved = saved.await;
                let _ = cx.update(|window, cx| {
                    // The board now edits the copy; say so, since the original stops changing.
                    let note = match saved {
                        Ok(_) => Notification::success(format!(
                            "Now editing “{copy}”. The original is unchanged."
                        ))
                        .title("Saved a copy"),
                        Err(cause) => Notification::error(cause),
                    };
                    window.push_notification(note, cx);
                });
            })
            .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(analysis) = self.analysis.upgrade() {
            analysis
                .update(cx, |analysis, cx| analysis.close_study(cx))
                .detach();
        }
    }

    /// Delete the open study; the board keeps its moves, and Undo brings the study back.
    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(analysis), Some((study, chapter))) = (self.analysis.upgrade(), self.open(cx))
        else {
            return;
        };
        let navigation = analysis.read(cx).navigation();
        let id = study["id"].as_str().unwrap_or_default().to_string();
        let library = Self::library(cx);
        let answer = library
            .read(cx)
            .command(json!({ "op": "remove", "id": id }));
        window
            .spawn(cx, async move |cx| {
                let result = answer.await;
                let _ = cx.update(|window, cx| {
                    let removed = match result {
                        Ok(result) => {
                            library.update(cx, |library, cx| {
                                library.apply(&result);
                                cx.notify();
                            });
                            result["removed"].clone()
                        }
                        Err(cause) => {
                            window.push_notification(Notification::error(cause), cx);
                            return;
                        }
                    };
                    if removed.is_null() {
                        return;
                    }
                    let name = removed["name"].as_str().unwrap_or_default().to_string();
                    let (library, analysis) = (library.clone(), analysis.clone());
                    let (id, chapter) = (id.clone(), chapter.clone());
                    window.push_notification(
                        crate::widgets::action_notification(
                            Notification::new()
                                .title(format!("Deleted “{name}”"))
                                .message("The board keeps its moves"),
                            "undo-delete-study",
                            "Undo",
                            move |window, cx| {
                                restore(
                                    &library,
                                    &analysis,
                                    removed.clone(),
                                    (id.clone(), chapter.clone(), navigation),
                                    window,
                                    cx,
                                );
                            },
                        ),
                        cx,
                    );
                });
            })
            .detach();
    }

    fn open_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(analysis) = self.analysis.upgrade() else {
            return;
        };
        let details = cx.new(|cx| GameDetails::new(analysis, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Game details")
                .w(px(512.))
                .child(details.clone())
        });
    }
}

/// Undo a deleted study, and link the board to it again while it still holds its moves.
fn restore(
    library: &Entity<crate::studies::StudyLibrary>,
    analysis: &Entity<AnalysisPage>,
    removed: Value,
    (id, chapter, navigation): (String, String, u64),
    window: &mut Window,
    cx: &mut App,
) {
    let answer = library
        .read(cx)
        .command(json!({ "op": "restore", "study": removed }));
    let (library, analysis) = (library.clone(), analysis.clone());
    window
        .spawn(cx, async move |cx| {
            let result = answer.await;
            let _ = cx.update(|window, cx| match result {
                Ok(result) => {
                    library.update(cx, |library, cx| {
                        library.apply(&result);
                        cx.notify();
                    });
                    analysis.update(cx, |analysis, cx| {
                        analysis.relink(id, chapter, navigation, cx)
                    });
                }
                Err(cause) => window.push_notification(Notification::error(cause), cx),
            });
        })
        .detach();
}

impl Render for StudyBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let palette = *cx.global::<Palette>();
        let open = self.open(cx);
        let error = self
            .analysis
            .upgrade()
            .and_then(|analysis| analysis.read(cx).study_error().map(str::to_string));
        let bar = cx.entity().downgrade();
        let menu_bar = bar.clone();
        let has_study = open.is_some();
        let cloud = open
            .as_ref()
            .is_some_and(|(study, _)| !study["cloud"].is_null());
        let menu = Button::new("study-actions")
            .ghost()
            .xsmall()
            .icon(icon("ellipsis"))
            .accessibility_label("Study actions")
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let action = |run: fn(&mut StudyBar, &mut Window, &mut Context<StudyBar>)| {
                    let bar = menu_bar.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = bar.update(cx, |bar, cx| run(bar, window, cx));
                    }
                };
                let menu = menu.item(
                    PopupMenuItem::new("Game details…")
                        .icon(icon("file-pen-line"))
                        .on_click(action(StudyBar::open_details)),
                );
                if !has_study {
                    return menu;
                }
                menu.separator()
                    .item(
                        PopupMenuItem::new("Save a copy")
                            .icon(icon("copy"))
                            .on_click(action(StudyBar::save_copy)),
                    )
                    .item(
                        PopupMenuItem::new("Close study")
                            .icon(icon("book-x"))
                            .on_click(action(|bar, _, cx| bar.close(cx))),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(if cloud {
                            "Remove offline copy"
                        } else {
                            "Delete study"
                        })
                        .icon(icon("trash-2"))
                        .on_click(action(StudyBar::remove)),
                    )
            });
        let left =
            match &open {
                Some((study, _)) => {
                    let count = study["chapters"].as_array().map_or(0, Vec::len);
                    let chapter_bar = bar.clone();
                    let chapter_menu = Button::new("chapter-actions")
                        .ghost()
                        .small()
                        .icon(icon("ellipsis"))
                        .accessibility_label("Chapter actions")
                        .dropdown_menu(move |menu, _, _| {
                            let action =
                                |run: fn(&mut StudyBar, &mut Window, &mut Context<StudyBar>)| {
                                    let bar = chapter_bar.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                        let _ = bar.update(cx, |bar, cx| run(bar, window, cx));
                                    }
                                };
                            menu.item(
                                PopupMenuItem::new("Add chapter")
                                    .icon(icon("plus"))
                                    .disabled(count >= MAX_CHAPTERS)
                                    .on_click(action(StudyBar::add_chapter)),
                            )
                            .item(
                                PopupMenuItem::new("Rename chapter")
                                    .icon(icon("pencil"))
                                    .on_click(action(StudyBar::rename_chapter)),
                            )
                            .item(
                                PopupMenuItem::new("Duplicate chapter")
                                    .icon(icon("copy"))
                                    .disabled(count >= MAX_CHAPTERS)
                                    .on_click(action(StudyBar::duplicate_chapter)),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("Delete chapter")
                                    .icon(icon("trash-2"))
                                    .disabled(count <= 1)
                                    .on_click(action(StudyBar::ask_remove_chapter)),
                            )
                        });
                    let saved = study["updatedAt"].as_f64().unwrap_or(0.);
                    h_flex()
                        .gap(px(8.))
                        .flex_wrap()
                        .child(
                            div()
                                .w(px(280.))
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.highlighted)
                                .child(
                                    Input::new(&self.name)
                                        .appearance(false)
                                        .small()
                                        .aria_label("Study name"),
                                ),
                        )
                        .child(
                            h_flex()
                                .w(px(260.))
                                .gap(px(4.))
                                .child(
                                    div().flex_1().min_w_0().child(
                                        Select::new(&self.chapters)
                                            .small()
                                            .accessibility_label("Study chapter"),
                                    ),
                                )
                                .child(chapter_menu),
                        )
                        .child(
                            h_flex()
                                .gap(px(4.))
                                .text_size(px(12.))
                                .text_color(if error.is_some() {
                                    palette.error
                                } else {
                                    palette.dimmed
                                })
                                .child(
                                    if error.is_some() {
                                        icon("triangle-alert")
                                    } else {
                                        icon("check")
                                    }
                                    .size(px(14.)),
                                )
                                .child(error.clone().unwrap_or_else(|| {
                                    format!("Saved {}", time_ago(saved, now()))
                                })),
                        )
                        .into_any_element()
                }
                None => h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.))
                            .text_color(palette.text_muted)
                            .child("Unsaved analysis"),
                    )
                    .child(
                        Button::new("save-as-study")
                            .flex_none()
                            .ghost()
                            .xsmall()
                            .icon(icon("book-plus"))
                            .label("Save as study")
                            .on_click(cx.listener(|this, _, window, cx| this.save_new(window, cx))),
                    )
                    .into_any_element(),
            };
        v_flex()
            .h(STUDY_BAR)
            .child(
                h_flex()
                    .min_h(px(36.))
                    .gap(px(8.))
                    .child(
                        if open.is_some() {
                            icon("book-open")
                        } else {
                            icon("book-dashed")
                        }
                        .size(px(18.))
                        .flex_none()
                        .text_color(palette.text_muted),
                    )
                    .child(div().flex_1().min_w_0().child(left))
                    .child(
                        Button::new("all-studies")
                            .ghost()
                            .xsmall()
                            .icon(icon("library-big"))
                            .label("All studies")
                            .on_click(|_, window, cx| {
                                crate::shell::open(Page::Studies, window, cx)
                            }),
                    )
                    .child(menu),
            )
            .children(self.chapter_error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .when(open.is_none() && error.is_some(), |bar| {
                bar.child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.error)
                        .child(error.clone().unwrap_or_default()),
                )
            })
    }
}

/// The game's details (`Game details…`): its players, event, date and result, edited in place.
struct GameDetails {
    fields: Vec<(&'static str, &'static str, Entity<InputState>)>,
    result: Entity<SelectState<Vec<SharedString>>>,
    _subscriptions: Vec<Subscription>,
}

impl GameDetails {
    fn new(analysis: Entity<AnalysisPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let fields: Vec<_> = [
            ("White", "White", ""),
            ("Black", "Black", ""),
            ("Event", "Event", ""),
            ("Date", "Date", "YYYY.MM.DD"),
        ]
        .into_iter()
        .map(|(key, label, placeholder)| {
            let value = analysis.read(cx).header(key);
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            });
            let target = analysis.clone();
            subscriptions.push(
                cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        target.update(cx, |analysis, cx| analysis.set_header(key, &value, cx));
                    }
                }),
            );
            (key, label, input)
        })
        .collect();
        let current = analysis.read(cx).header("Result");
        let chosen = RESULTS
            .iter()
            .position(|result| *result == current)
            .unwrap_or(0);
        let result = cx.new(|cx| {
            SelectState::new(
                RESULTS
                    .iter()
                    .map(|result| SharedString::from(*result))
                    .collect(),
                Some(gpui_kit::component::IndexPath::new(chosen)),
                window,
                cx,
            )
        });
        let target = analysis.clone();
        subscriptions.push(cx.subscribe(
            &result,
            move |_, _, event: &SelectEvent<Vec<SharedString>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let value = value.to_string();
                    target.update(cx, |analysis, cx| analysis.set_header("Result", &value, cx));
                }
            },
        ));
        GameDetails {
            fields,
            result,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for GameDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let field = |label: &'static str, control: AnyElement| {
            v_flex()
                .flex_1()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child(label),
                )
                .child(control)
        };
        let mut cells: Vec<AnyElement> = self
            .fields
            .iter()
            .map(|(_, label, input)| {
                field(
                    label,
                    Input::new(input).aria_label(*label).into_any_element(),
                )
                .into_any_element()
            })
            .collect();
        cells.push(
            field(
                "Result",
                Select::new(&self.result)
                    .accessibility_label("Result")
                    .into_any_element(),
            )
            .into_any_element(),
        );
        let mut rows = Vec::new();
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some() {
            let row: Vec<AnyElement> = cells.by_ref().take(2).collect();
            let alone = row.len() == 1;
            rows.push(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .children(row)
                    .when(alone, |row| row.child(div().flex_1())),
            );
        }
        v_flex().gap(px(12.)).children(rows)
    }
}
