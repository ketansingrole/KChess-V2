//! Studies (`pages/studies.vue`): the study library on this computer. Browse, search and open saved
//! studies on the analysis board, start or import one, and rename, copy, drill, upload or delete
//! them. Lichess's own studies are a tab of their own when an account is connected.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::Color;

use super::Page;
use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::tree::TreeNode;
use crate::name_dialog::NameDialog;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::studies::{
    MAX_CHAPTERS, MAX_STUDIES, StudyHandle, StudyLibrary, document_pgn, now, time_ago,
};
use crate::theme::Palette;
mod lichess;

use crate::widgets::{self, Pill};

const TAB: &str = "kchess:studies-tab";
const SORT: &str = "kchess:studies-sort";
const SORTS: [(&str, &str); 2] = [("recent", "Recently edited"), ("name", "Name")];
/// The cards' columns (`.card-grid`).
const CARD_MIN: Pixels = px(210.);
const CARD_GAP: Pixels = px(16.);
/// "Edited 2 min ago" is redrawn this often.
const CLOCK: Duration = Duration::from_secs(60);

/// What a study's card shows of it (`summarizeStudy`): its final position, players and event.
fn summary(pgn: &str) -> Value {
    crate::game::rules("summarizeStudy", &[json!(pgn)]).unwrap_or(Value::Null)
}

/// `localeCompare` with numbers read as numbers ("Study 2" before "Study 10"), ignoring case.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(digit) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(digit);
                        chars.next();
                    }
                    digits.trim_start_matches('0').to_string()
                };
                let (x, y) = (number(&mut a), number(&mut b));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// An import's chapters (`pgnGames`): each game named by its `ChapterName`, or numbered.
fn import_chapters(text: &str) -> Option<Vec<(String, String)>> {
    let games = crate::game::rules("pgnGames", &[json!(text)]).ok()?;
    let chapters: Vec<(String, String)> = games
        .as_array()?
        .iter()
        .enumerate()
        .map(|(index, game)| {
            let name = game["headers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|pair| pair[0] == json!("ChapterName"))
                .and_then(|pair| pair[1].as_str())
                .map_or_else(|| format!("Chapter {}", index + 1), str::to_string);
            (name, game["pgn"].as_str().unwrap_or_default().to_string())
        })
        .collect();
    let valid = !chapters.is_empty()
        && chapters.len() <= MAX_CHAPTERS
        && chapters
            .iter()
            .all(|(_, pgn)| TreeNode::from_pgn(pgn).is_some());
    valid.then_some(chapters)
}

/// An imported study's name when none is given: its players, else its event.
fn import_name(first: &str) -> Option<String> {
    let headers = TreeNode::from_pgn(first)?.headers.unwrap_or_default();
    let header = |key: &str| {
        headers
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && *value != "?")
            .map(str::to_string)
    };
    match (header("White"), header("Black")) {
        (Some(white), Some(black)) => Some(format!("{white} – {black}")),
        _ => header("Event"),
    }
}

pub struct StudiesPage {
    search: Entity<InputState>,
    sort: Entity<SelectState<Vec<SharedString>>>,
    /// The Lichess tab: the account's studies and what is being done with them.
    lichess: lichess::LichessTab,
    uploading: Option<String>,
    previews: HashMap<String, Entity<ChessBoard>>,
    area: Measured,
    focus: FocusHandle,
    _clock: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for StudiesPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl StudiesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> StudiesPage {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search by name, players or event"));
        let sort_key = crate::prefs::get::<String>(cx, SORT, "recent".into());
        let sort = cx.new(|cx| {
            SelectState::new(
                SORTS
                    .iter()
                    .map(|(_, label)| SharedString::from(*label))
                    .collect(),
                Some(IndexPath::new(
                    SORTS
                        .iter()
                        .position(|(key, _)| *key == sort_key)
                        .unwrap_or(0),
                )),
                window,
                cx,
            )
        });
        let library = cx.global::<StudyHandle>().0.clone();
        let (lichess, lichess_subscriptions) = lichess::new_tab(window, cx);
        let mut subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&sort, |_, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                let row = state
                    .read(cx)
                    .selected_index(cx)
                    .map_or(0, |index| index.row);
                crate::prefs::set(cx, SORT, SORTS[row.min(1)].0);
                cx.notify();
            }),
            cx.observe(&library, |_, _, cx| cx.notify()),
            cx.observe(&crate::accounts::handle(cx), |_, _, cx| cx.notify()),
        ];
        subscriptions.extend(lichess_subscriptions);
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut page = StudiesPage {
            search,
            sort,
            lichess,
            uploading: None,
            previews: HashMap::new(),
            area: Measured::default(),
            focus: cx.focus_handle(),
            _clock: clock,
            _subscriptions: subscriptions,
        };
        page.shown(cx);
        page
    }

    /// The page was opened: the Lichess tab's list, when it is the one shown.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        if crate::prefs::get::<String>(cx, TAB, "device".into()) == "lichess" {
            self.load_lichess(false, cx);
        }
    }

    fn library(cx: &App) -> Entity<StudyLibrary> {
        cx.global::<StudyHandle>().0.clone()
    }

    fn connected(cx: &App) -> bool {
        !crate::accounts::handle(cx).read(cx).connected().is_empty()
    }

    /// A small board picturing a position, made once per position.
    fn preview(&mut self, fen: &str, cx: &mut Context<Self>) -> Entity<ChessBoard> {
        self.previews
            .entry(fen.to_string())
            .or_insert_with(|| {
                let position =
                    kchess_domain::rules::setup_start("standard", fen).unwrap_or_default();
                let mut state = BoardState::of(&position, Color::White, None, None);
                state.preview = true;
                cx.new(|_| ChessBoard::new(state))
            })
            .clone()
    }

    /* ── Opening and starting studies ─────────────────────────────────── */

    /// Show a study on the analysis board; never silently throw away an unsaved board.
    fn open(id: String, window: &mut Window, cx: &mut App) {
        let shown = crate::shell::analysis_board(window, cx)
            .is_some_and(|analysis| analysis.read(cx).study_id() == id);
        if shown {
            crate::shell::open(Page::Analysis, window, cx);
            return;
        }
        crate::shell::replace_analysis(window, cx, move |analysis, window, cx| {
            match analysis.open_study(&id, None, cx) {
                Ok(()) => true,
                Err(cause) => {
                    window.push_notification(Notification::error(cause), cx);
                    false
                }
            }
        });
    }

    /// `New study`: an empty board, kept as a study at once.
    fn create(window: &mut Window, cx: &mut App) {
        let name = Self::library(cx).read(cx).fresh_name("Untitled study");
        crate::shell::replace_analysis(window, cx, move |analysis, window, cx| {
            analysis.load(None, cx);
            let saved = analysis.save_as_study(name.clone(), cx);
            window
                .spawn(cx, async move |cx| {
                    if let Err(cause) = saved.await {
                        crate::log::write(
                            Level::Warn,
                            "studies",
                            &format!("Could not save study: {cause}"),
                        );
                        let _ = cx.update(|window, cx| {
                            window.push_notification(Notification::error(cause), cx)
                        });
                    }
                })
                .detach();
            true
        });
    }

    /// Practice → Repertoire on this study.
    fn drill(id: String, window: &mut Window, cx: &mut App) {
        crate::prefs::set(cx, "kchess:repertoire-study", id);
        crate::prefs::set(cx, "kchess:practice-tab", "openings");
        crate::shell::open(Page::Practice, window, cx);
    }

    /// Send a command and show its answer; a failure is a notification.
    fn run(
        command: Value,
        window: &mut Window,
        cx: &mut App,
        then: impl FnOnce(Value, &mut Window, &mut App) + 'static,
    ) {
        let library = Self::library(cx);
        let answer = library.read(cx).command(command);
        window
            .spawn(cx, async move |cx| {
                let result = answer.await;
                let _ = cx.update(|window, cx| match result {
                    Ok(result) => {
                        library.update(cx, |library, cx| {
                            library.apply(&result);
                            cx.notify();
                        });
                        then(result, window, cx);
                    }
                    Err(cause) => window.push_notification(Notification::error(cause), cx),
                });
            })
            .detach();
    }

    fn rename(study: &Value, window: &mut Window, cx: &mut App) {
        let id = study["id"].clone();
        let library = Self::library(cx);
        NameDialog::open(
            "Rename study",
            study["name"].as_str().unwrap_or_default(),
            "Rename",
            window,
            cx,
            move |name, dialog, window, cx| {
                let answer = library
                    .read(cx)
                    .command(json!({ "op": "rename", "id": id, "name": name }));
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

    /// Delete a study (or its offline copy); Undo brings it back.
    fn remove(id: Value, window: &mut Window, cx: &mut App) {
        Self::run(
            json!({ "op": "remove", "id": id }),
            window,
            cx,
            |result, window, cx| {
                let removed = result["removed"].clone();
                if removed.is_null() {
                    return;
                }
                let name = removed["name"].as_str().unwrap_or_default().to_string();
                window.push_notification(
                    widgets::action_notification(
                        Notification::new().title(format!("Deleted “{name}”")),
                        "undo-delete",
                        "Undo",
                        move |window, cx| {
                            Self::run(
                                json!({ "op": "restore", "study": removed.clone() }),
                                window,
                                cx,
                                |_, _, _| {},
                            );
                        },
                    ),
                    cx,
                );
            },
        );
    }

    fn copy_pgn(study: &Value, window: &mut Window, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(document_pgn(study)));
        window.push_notification("PGN copied", cx);
    }

    /// Upload a study to Lichess: its changes to the cloud copy it came from, or as a new study.
    fn upload(&mut self, id: String, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = crate::accounts::handle(cx).read(cx).active(cx) else {
            window.push_notification(
                Notification::warning("Connect Lichess to upload a study"),
                cx,
            );
            return;
        };
        if self.uploading.is_some() {
            return;
        }
        let library = Self::library(cx);
        let Some(study) = library.read(cx).study(&id).cloned() else {
            return;
        };
        let cloud = study["cloud"].clone();
        let cloud_account = cloud["account"].as_str().unwrap_or_default();
        if !copy && !cloud.is_null() && !cloud_account.eq_ignore_ascii_case(&account) {
            window.push_notification(
                Notification::warning(format!(
                    "Switch to @{cloud_account} to upload changes to this study."
                )),
                cx,
            );
            return;
        }
        self.uploading = Some(id.clone());
        cx.notify();
        // The board's last edits first, when it shows this study.
        let flushed = crate::shell::analysis_board(window, cx)
            .map(|analysis| analysis.update(cx, |analysis, cx| analysis.flush_study(cx)));
        let core = cx.global::<AppCore>().clone();
        cx.spawn_in(window, async move |this, cx| {
            let outcome: Result<(), String> = async {
                if let Some(flushed) = flushed {
                    flushed.await?;
                }
                let study = this
                    .update(cx, |_, cx| library.read(cx).study(&id).cloned())
                    .ok()
                    .flatten()
                    .ok_or("The study is gone.")?;
                let pgn = document_pgn(&study);
                let cloud = study["cloud"].clone();
                let linked = !copy
                    && !cloud.is_null()
                    && cloud["structureChanged"] != json!(true)
                    && cloud["account"]
                        .as_str()
                        .is_some_and(|owner| owner.eq_ignore_ascii_case(&account));
                let result = if linked {
                    core.call(
                        "syncLichessStudy",
                        vec![json!({
                            "account": account,
                            "studyId": cloud["id"],
                            "baseline": cloud["downloadedPgn"],
                            "pgn": pgn,
                        })],
                    )
                    .await
                } else {
                    let name: String = study["name"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(100)
                        .collect();
                    core.call(
                        "exportToLichessStudy",
                        vec![json!(account), json!(""), json!(name), json!(pgn)],
                    )
                    .await
                }
                .map_err(|cause| cause.message)?;
                if result.get("needsReconnect").is_some() {
                    return Err(format!("Connect @{account} again to allow study access."));
                }
                let remote = if linked {
                    cloud["id"].as_str().unwrap_or_default().to_string()
                } else {
                    result["id"].as_str().unwrap_or_default().to_string()
                };
                let chapters = if result.is_array() {
                    result
                } else {
                    core.call("lichessStudyChapters", vec![json!(account), json!(remote)])
                        .await
                        .map_err(|cause| cause.message)?
                };
                if chapters.get("needsReconnect").is_some() {
                    return Err("Uploaded, but study access needs reconnection.".into());
                }
                let baseline = chapters
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|chapter| chapter["pgn"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let marked = this
                    .update(cx, |_, cx| {
                        library.read(cx).command(json!({
                            "op": "markCloud",
                            "id": id,
                            "account": account,
                            "remoteId": remote,
                            "baseline": baseline,
                        }))
                    })
                    .map_err(|_| "The page closed.".to_string())?;
                let result = marked.await?;
                let _ = this.update(cx, |_, cx| {
                    library.update(cx, |library, cx| {
                        library.apply(&result);
                        cx.notify();
                    })
                });
                Ok(())
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.uploading = None;
                match outcome {
                    Ok(()) => window.push_notification(Notification::success("Study uploaded"), cx),
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "studies",
                            &format!("Study upload failed: {cause}"),
                        );
                        window.push_notification(Notification::error(cause), cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_import(window: &mut Window, cx: &mut App) {
        let import = cx.new(|cx| ImportStudy::new(window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Import a study")
                .w(px(512.))
                .child(import.clone())
        });
    }

    /* ── Rendering ────────────────────────────────────────────────────── */

    fn header_actions() -> impl IntoElement {
        h_flex()
            .gap(px(8.))
            .child(
                Button::new("import-pgn")
                    .outline()
                    .icon(icon("import"))
                    .label("Import PGN")
                    .on_click(|_, window, cx| Self::open_import(window, cx)),
            )
            .child(
                Button::new("new-study")
                    .primary()
                    .icon(icon("plus"))
                    .label("New study")
                    .on_click(|_, window, cx| Self::create(window, cx)),
            )
    }

    fn menu(&self, study: &Value) -> impl IntoElement {
        let id = study["id"].as_str().unwrap_or_default().to_string();
        let name = study["name"].as_str().unwrap_or_default().to_string();
        let cloud = !study["cloud"].is_null();
        let can_copy = cloud && study["cloud"]["structureChanged"] != json!(true);
        let busy = self.uploading.is_some();
        let study = study.clone();
        Button::new(SharedString::from(format!("study-menu-{id}")))
            .ghost()
            .xsmall()
            .icon(icon("ellipsis-vertical"))
            .accessibility_label(format!("Actions for {name}"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let (open, drill, rename, duplicate, copy, upload, remove) = (
                    id.clone(),
                    id.clone(),
                    study.clone(),
                    id.clone(),
                    study.clone(),
                    id.clone(),
                    id.clone(),
                );
                let menu = menu
                    .item(
                        PopupMenuItem::new("Open")
                            .icon(icon("square-arrow-out-up-right"))
                            .on_click(move |_, window, cx| Self::open(open.clone(), window, cx)),
                    )
                    .item(
                        PopupMenuItem::new("Drill as repertoire")
                            .icon(icon("graduation-cap"))
                            .on_click(move |_, window, cx| Self::drill(drill.clone(), window, cx)),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Rename…")
                            .icon(icon("pencil"))
                            .on_click(move |_, window, cx| Self::rename(&rename, window, cx)),
                    )
                    .item(PopupMenuItem::new("Duplicate").icon(icon("copy")).on_click(
                        move |_, window, cx| {
                            Self::run(
                                json!({ "op": "duplicate", "id": duplicate }),
                                window,
                                cx,
                                |_, _, _| {},
                            )
                        },
                    ))
                    .item(
                        PopupMenuItem::new("Copy PGN")
                            .icon(icon("file-text"))
                            .on_click(move |_, window, cx| Self::copy_pgn(&copy, window, cx)),
                    );
                let menu = if can_copy {
                    menu.item(
                        PopupMenuItem::new("Upload a new cloud copy")
                            .icon(icon("cloud-upload"))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                if let Some(page) =
                                    crate::shell::view_of::<StudiesPage>(Page::Studies, cx)
                                {
                                    let id = upload.clone();
                                    page.update(cx, |page, cx| page.upload(id, true, window, cx));
                                }
                            }),
                    )
                } else {
                    menu
                };
                menu.separator().item(
                    PopupMenuItem::new(if cloud {
                        "Remove offline copy"
                    } else {
                        "Delete"
                    })
                    .icon(icon("trash-2"))
                    .on_click(move |_, window, cx| Self::remove(json!(remove.clone()), window, cx)),
                )
            })
    }

    fn render_cards(
        &mut self,
        studies: Vec<Value>,
        width: Pixels,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let now = now();
        let cards: Vec<AnyElement> = studies
            .iter()
            .map(|study| {
                let id = study["id"].as_str().unwrap_or_default().to_string();
                let name = study["name"].as_str().unwrap_or_default().to_string();
                let summary = summary(study["pgn"].as_str().unwrap_or_default());
                let board = summary["fen"]
                    .as_str()
                    .map(|fen| self.preview(fen, cx).into());
                let event = summary["event"].as_str().filter(|event| *event != name);
                let description = [summary["players"].as_str(), event]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                let chapters = study["chapters"].as_array().map_or(0, Vec::len);
                let cloud = &study["cloud"];
                let location = format!(
                    "{}{}",
                    if cloud.is_null() {
                        "On this device"
                    } else {
                        "Offline · Lichess"
                    },
                    if chapters > 0 {
                        format!(
                            " · {chapters} {}",
                            if chapters == 1 { "chapter" } else { "chapters" }
                        )
                    } else {
                        String::new()
                    }
                );
                let upload_label = if cloud["structureChanged"] == json!(true) {
                    "Upload as new cloud study"
                } else if cloud.is_null() {
                    "Upload to Lichess"
                } else {
                    "Upload changes"
                };
                let uploading = self.uploading.as_deref() == Some(id.as_str());
                let page = cx.entity().downgrade();
                let upload_id = id.clone();
                let open_id = id.clone();
                widgets::study_card(
                    widgets::StudyCard {
                        id: id.clone(),
                        name,
                        board,
                        location,
                        description: (!description.is_empty()).then_some(description),
                        edited: study["updatedAt"]
                            .as_f64()
                            .filter(|at| *at > 0.)
                            .map(|at| format!("Edited {}", time_ago(at, now))),
                        rows: 3,
                        menu: Some(self.menu(study).into_any_element()),
                        action: Button::new(SharedString::from(format!("upload-{id}")))
                            .outline()
                            .xsmall()
                            .w_full()
                            .icon(icon("cloud-upload"))
                            .label(upload_label)
                            .loading(uploading)
                            .disabled(self.uploading.is_some())
                            .on_click(move |_, window, cx| {
                                let _ = page.update(cx, |page, cx| {
                                    page.upload(upload_id.clone(), false, window, cx)
                                });
                            })
                            .into_any_element(),
                    },
                    card_width,
                    move |window, cx| Self::open(open_id.clone(), window, cx),
                    palette,
                )
            })
            .collect();
        let mut rows = Vec::new();
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            rows.push(
                h_flex()
                    .items_stretch()
                    .gap(CARD_GAP)
                    .children(cards.by_ref().take(count)),
            );
        }
        v_flex().gap(CARD_GAP).children(rows).into_any_element()
    }

    fn render_device(
        &mut self,
        width: Pixels,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let library = Self::library(cx);
        let (items, error) = {
            let library = library.read(cx);
            (library.items().to_vec(), library.error.clone())
        };
        let error_line = error.clone().map(|error| {
            div()
                .mb(px(12.))
                .text_size(px(14.))
                .text_color(palette.error)
                .child(error)
        });
        if items.is_empty() {
            return v_flex()
                .children(error_line)
                .when(error.is_none(), |page| {
                    page.child(
                        widgets::card(palette)
                            .items_center()
                            .gap(px(12.))
                            .py(px(48.))
                            .px(px(24.))
                            .child(icon("library-big").size(px(40.)).text_color(palette.dimmed))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("No studies yet"),
                            )
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .child(
                                        Button::new("empty-new-study")
                                            .primary()
                                            .icon(icon("plus"))
                                            .label("New study")
                                            .on_click(|_, window, cx| Self::create(window, cx)),
                                    )
                                    .child(
                                        Button::new("empty-import")
                                            .outline()
                                            .icon(icon("import"))
                                            .label("Import PGN")
                                            .on_click(|_, window, cx| {
                                                Self::open_import(window, cx)
                                            }),
                                    ),
                            ),
                    )
                })
                .into_any_element();
        }
        let query = self.search.read(cx).value().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let by_name = crate::prefs::get::<String>(cx, SORT, "recent".into()) == "name";
        let mut studies: Vec<Value> = items
            .iter()
            .filter(|study| {
                let summary = summary(study["pgn"].as_str().unwrap_or_default());
                let text = format!(
                    "{} {} {}",
                    study["name"].as_str().unwrap_or_default(),
                    summary["players"].as_str().unwrap_or_default(),
                    summary["event"].as_str().unwrap_or_default()
                )
                .to_lowercase();
                words.iter().all(|word| text.contains(word))
            })
            .cloned()
            .collect();
        if by_name {
            studies.sort_by(|a, b| {
                natural_cmp(
                    a["name"].as_str().unwrap_or_default(),
                    b["name"].as_str().unwrap_or_default(),
                )
            });
        } else {
            studies.sort_by(|a, b| {
                b["updatedAt"]
                    .as_f64()
                    .unwrap_or(0.)
                    .total_cmp(&a["updatedAt"].as_f64().unwrap_or(0.))
            });
        }
        let toolbar = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .mb(px(16.))
            .child(
                div().flex_1().min_w(px(192.)).child(
                    Input::new(&self.search)
                        .prefix(icon("search"))
                        .aria_label("Search studies"),
                ),
            )
            .child(
                div()
                    .w(px(176.))
                    .child(Select::new(&self.sort).accessibility_label("Sort studies")),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(format!("{} of {MAX_STUDIES}", items.len())),
            );
        let body = if studies.is_empty() {
            div()
                .py(px(40.))
                .text_center()
                .text_color(palette.text_muted)
                .child(format!(
                    "No study matches “{}”.",
                    self.search.read(cx).value()
                ))
                .into_any_element()
        } else {
            self.render_cards(studies, width, cx, palette)
        };
        v_flex()
            .child(toolbar)
            .children(error_line)
            .child(body)
            .into_any_element()
    }
}

impl Render for StudiesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let connected = Self::connected(cx);
        let tab = if connected {
            crate::prefs::get::<String>(cx, TAB, "device".into())
        } else {
            "device".into()
        };
        let count = Self::library(cx).read(cx).items().len();
        let width = self.area.size().width;
        let width = if width > px(0.) {
            width
        } else {
            window.viewport_size().width - px(400.)
        };
        let body = if tab == "lichess" {
            self.render_lichess(width, window, cx, &palette)
        } else {
            self.render_device(width, cx, &palette)
        };
        div()
            .id("studies")
            .size_full()
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .when(count > 0 || tab == "lichess", |page| {
                        page.child(
                            h_flex()
                                .justify_end()
                                .mb(px(20.))
                                .child(Self::header_actions()),
                        )
                    })
                    .when(connected, |page| {
                        page.child(div().mb(px(20.)).child(widgets::pills(
                            "studies-tabs",
                            vec![
                                Pill {
                                    icon: Some(icon("hard-drive")),
                                    label: format!("On this device · {count}").into(),
                                    badge: None,
                                    disabled: false,
                                },
                                Pill {
                                    icon: Some(icon("cloud")),
                                    label: "Lichess".into(),
                                    badge: None,
                                    disabled: false,
                                },
                            ],
                            usize::from(tab == "lichess"),
                            {
                                let page = cx.entity().downgrade();
                                move |index, window, cx| {
                                    crate::prefs::set(
                                        cx,
                                        TAB,
                                        if index == 1 { "lichess" } else { "device" },
                                    );
                                    if index == 1 {
                                        let _ = page
                                            .update(cx, |page, cx| page.load_lichess(false, cx));
                                    }
                                    window.refresh();
                                }
                            },
                            false,
                            &palette,
                        )))
                    })
                    .child(div().relative().child(self.area.probe(cx)).child(body)),
            )
    }
}

/// `Import a study`: a name (taken from the PGN when left empty) and PGN chapters.
struct ImportStudy {
    name: Entity<InputState>,
    text: Entity<TextareaState>,
    error: Option<SharedString>,
    _subscription: Subscription,
}

impl ImportStudy {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> ImportStudy {
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Name (optional: taken from the PGN)")
        });
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder("[Event \"…\"]\n1. e4 e5 2. Nf3 { The main idea } Nc6 (2... d6) *")
        });
        let subscription = cx.observe(&text, |_, _, cx| cx.notify());
        ImportStudy {
            name,
            text,
            error: None,
            _subscription: subscription,
        }
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).value().trim().to_string();
        let Some(chapters) = import_chapters(&text) else {
            self.error = Some("Paste valid PGN with up to 64 chapters.".into());
            cx.notify();
            return;
        };
        let library = StudiesPage::library(cx);
        let given = self.name.read(cx).value().trim().to_string();
        let name = if given.is_empty() {
            import_name(&chapters[0].1)
                .unwrap_or_else(|| library.read(cx).fresh_name("Imported study"))
        } else {
            given
        };
        let answer = library.read(cx).command(json!({
            "op": "saveChapters",
            "name": name,
            "chapters": chapters
                .iter()
                .map(|(name, pgn)| json!({ "name": name, "pgn": pgn }))
                .collect::<Vec<_>>(),
        }));
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(result) => {
                    library.update(cx, |library, cx| {
                        library.apply(&result);
                        cx.notify();
                    });
                    window.close_dialog(cx);
                    let id = result["id"].as_str().unwrap_or_default().to_string();
                    window.push_notification(
                        widgets::action_notification(
                            Notification::success("Study imported"),
                            "open-imported",
                            "Open",
                            move |window, cx| StudiesPage::open(id.clone(), window, cx),
                        ),
                        cx,
                    );
                }
                Err(cause) => {
                    crate::log::write(Level::Warn, "studies", &format!("Import failed: {cause}"));
                    this.error = Some(cause.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for ImportStudy {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.text.read(cx).value().trim().is_empty();
        v_flex()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(palette.text_muted)
                    .child("Paste PGN chapters."),
            )
            .child(Input::new(&self.name).aria_label("Study name"))
            .child(Textarea::new(&self.text).aria_label("PGN"))
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(14.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("import-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("import-run")
                            .primary()
                            .label("Import")
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.run(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{import_chapters, import_name, natural_cmp};
    use std::cmp::Ordering;

    #[test]
    fn names_sort_with_their_numbers() {
        assert_eq!(natural_cmp("Study 2", "Study 10"), Ordering::Less);
        assert_eq!(natural_cmp("alpha", "Beta"), Ordering::Less);
        assert_eq!(natural_cmp("Study 7", "study 7"), Ordering::Equal);
    }

    #[test]
    fn an_import_names_its_chapters() {
        let text = "[Event \"Club\"]\n[White \"A\"]\n[Black \"B\"]\n[ChapterName \"Main\"]\n\n1. e4 e5 *\n\n[Event \"Club\"]\n\n1. d4 *";
        let chapters = import_chapters(text).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].0, "Main");
        assert_eq!(chapters[1].0, "Chapter 2");
        assert_eq!(import_name(&chapters[0].1).as_deref(), Some("A – B"));
        assert!(import_chapters("").is_none());
        // A study holds at most 64 chapters.
        assert!(import_chapters(&"[Event \"?\"]\n\n1. e4 *\n\n".repeat(65)).is_none());
    }
}
