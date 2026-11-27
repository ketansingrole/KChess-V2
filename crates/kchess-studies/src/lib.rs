//! The Studies page: the study library on this computer. Browse, search and open saved
//! studies on the analysis board, start or import one, and rename, copy, drill, upload or delete
//! them. Lichess's own studies are a tab of their own when an account is connected.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// The app's services, the design system and the board, under the names this page's modules use.
// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

use kchess_app::{accounts, core, log, prefs, studies, timer};
use kchess_board::{board, export, game, widgets};
use kchess_ui::{name_dialog, theme};

use kchess_contracts::calls;
use kchess_contracts::check::{ChapterName, LichessId, PgnText, StudyId, Username};
use kchess_contracts::types::{Reply, StudyExport, StudySyncRequest};
use kchess_nav::page::PageView;
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
use kchess_contracts::Level;
use kchess_contracts::types::{ChapterInput, Study, StudyCommand, StudyCommandResult};
use kchess_domain::records::studies::StudySummary;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::tree::TreeNode;
use crate::name_dialog::NameDialog;
use crate::studies::{MAX_CHAPTERS, MAX_STUDIES, StudyHandle, StudyLibrary, document_pgn};
use crate::theme::Palette;
use crate::widgets::time_ago;
use kchess_games::now_ms;
use kchess_nav::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use kchess_ui::icon;
mod grid;
mod import;
mod lichess;

pub use grid::estimated_row;
use grid::*;
use import::*;

use crate::widgets::Pill;

const TAB: &str = "kchess:studies-tab";
const SORT: &str = "kchess:studies-sort";
const SORTS: [(&str, &str); 2] = [("recent", "Recently edited"), ("name", "Name")];
/// The cards' columns.
const CARD_MIN: Pixels = px(210.);
const CARD_GAP: Pixels = px(16.);
/// "Edited 2 min ago" is redrawn this often.
const CLOCK: Duration = Duration::from_secs(60);

/// What a study's card shows of it: its final position, players and event; none for a PGN that
/// cannot be imported.
fn summary(pgn: &str) -> Option<StudySummary> {
    kchess_domain::records::studies::summarize_study(pgn)
}

/// Alphabetical, with numbers read as numbers ("Study 2" before "Study 10"), ignoring case.
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

pub struct StudiesPage {
    search: Entity<InputState>,
    sort: Entity<SelectState<Vec<SharedString>>>,
    /// The Lichess tab: the account's studies and what is being done with them.
    lichess: lichess::LichessTab,
    uploading: Option<String>,
    previews: crate::board::Previews,
    /// The page's scrolling, and where the cards' grid lies in it: only the rows of cards in view
    /// (and a margin around it) are made, laid out and drawn. Fifty cards, the library's limit,
    /// were most of opening the page.
    scroll: ScrollHandle,
    grid: Measured,
    /// Each row of cards' height as it was last laid out, and the cards' width then; a row not yet
    /// made is taken to be as tall as the first measured.
    row_heights: std::rc::Rc<std::cell::RefCell<RowHeights>>,
    /// Each listed study's summary by its PGN: reading it parses the study and plays it out, and
    /// every frame of the page asks for each study's twice.
    summaries: std::cell::RefCell<HashMap<String, Option<StudySummary>>>,
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
            cx.observe_in(
                &crate::accounts::handle(cx),
                window,
                |this, _, window, cx| {
                    this.sync_lichess_account(window, cx);
                    cx.notify();
                },
            ),
        ];
        subscriptions.extend(lichess_subscriptions);
        let clock = crate::timer::redraw_every(CLOCK, cx);
        let mut page = StudiesPage {
            search,
            sort,
            lichess,
            uploading: None,
            previews: Default::default(),
            scroll: ScrollHandle::new(),
            grid: Measured::default(),
            row_heights: Default::default(),
            summaries: Default::default(),
            area: Measured::default(),
            focus: cx.focus_handle(),
            _clock: clock,
            _subscriptions: subscriptions,
        };
        page.sync_lichess_account(window, cx);
        page
    }

    /// The page was opened: the Lichess tab's list, when it is the one shown.
    pub fn shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::prefs::get::<String>(cx, TAB, "device".into()) == "lichess" {
            self.load_lichess(false, window, cx);
        }
    }

    fn library(cx: &App) -> Entity<StudyLibrary> {
        cx.global::<StudyHandle>().0.clone()
    }

    /// Show the Lichess tab, as its pill does.
    pub fn show_lichess(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, TAB, "lichess");
        self.load_lichess(false, window, cx);
        window.refresh();
    }

    fn connected(cx: &App) -> bool {
        !crate::accounts::handle(cx).read(cx).connected().is_empty()
    }

    /// A small board picturing a position in a study's card, made once per position.
    fn preview(&mut self, fen: &str, cx: &mut Context<Self>) -> Entity<ChessBoard> {
        let state = || BoardState {
            corners: widgets::card_picture_corners(),
            ..BoardState::preview(fen, Default::default(), Color::White, None)
        };
        self.previews.board(fen.to_string(), state, cx)
    }

    /* ── Opening and starting studies ─────────────────────────────────── */

    /// Show a study on the analysis board; never silently throw away an unsaved board.
    fn open(id: String, window: &mut Window, cx: &mut App) {
        kchess_board::analysis::open(kchess_board::analysis::Open::Study(id), window, cx);
    }

    /// `New study`: an empty board, kept as a study at once.
    pub(crate) fn create(window: &mut Window, cx: &mut App) {
        let name = Self::library(cx).read(cx).fresh_name("Untitled study");
        kchess_board::analysis::open(kchess_board::analysis::Open::NewStudy(name), window, cx);
    }

    /// Practice → Repertoire on this study.
    fn drill(id: String, window: &mut Window, cx: &mut App) {
        kchess_practice::repertoire::train_study(id, window, cx);
    }

    /// Send a command and show its answer; a failure is a notification.
    fn run(
        command: StudyCommand,
        window: &mut Window,
        cx: &mut App,
        then: impl FnOnce(StudyCommandResult, &mut Window, &mut App) + 'static,
    ) {
        let library = Self::library(cx);
        let answer = library.read(cx).command(command);
        window
            .spawn(cx, async move |cx| {
                let result = answer.await;
                cx.update(|window, cx| match result {
                    Ok(result) => {
                        library.update(cx, |library, cx| {
                            library.apply(&result);
                            cx.notify();
                        });
                        then(result, window, cx);
                    }
                    Err(cause) => window.push_notification(Notification::error(cause), cx),
                })
                .ok();
            })
            .detach();
    }

    fn rename(study: &Study, window: &mut Window, cx: &mut App) {
        let id = study.id.clone();
        ask_name(
            "Rename study",
            &study.name,
            "Rename",
            window,
            cx,
            move |name| StudyCommand::Rename {
                id: id.clone(),
                name,
            },
        );
    }

    /// Delete a study (or its offline copy); Undo brings it back.
    fn remove(id: String, window: &mut Window, cx: &mut App) {
        Self::run(
            StudyCommand::Remove { id },
            window,
            cx,
            |result, window, cx| {
                let Some(removed) = result.removed else {
                    return;
                };
                let name = removed.name.clone();
                window.push_notification(
                    widgets::action_notification(
                        Notification::new().title(format!("Deleted “{name}”")),
                        "undo-delete",
                        "Undo",
                        move |window, cx| {
                            Self::run(
                                StudyCommand::Restore {
                                    study: json!(removed),
                                },
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

    fn copy_pgn(study: &Study, window: &mut Window, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(document_pgn(study)));
        window.push_notification("PGN copied", cx);
    }

    /// Save a study's PGN, every chapter, as a file where the player picks.
    fn save_pgn(study: &Study, window: &mut Window, cx: &mut App) {
        let name: String = study
            .name
            .chars()
            .filter(|c| c.is_alphanumeric() || " _.()+-".contains(*c))
            .take(80)
            .collect();
        let name = if name.trim().is_empty() {
            "study".to_string()
        } else {
            name.trim().to_string()
        };
        let saved = crate::export::save_text(&format!("{name}.pgn"), document_pgn(study), cx);
        window
            .spawn(cx, async move |cx| {
                let note = match saved.await {
                    Some(Ok(())) => Notification::new().title("PGN saved"),
                    Some(Err(cause)) => {
                        Notification::error(format!("Couldn't save the PGN: {cause}"))
                    }
                    None => return,
                };
                cx.update(|window, cx| window.push_notification(note, cx))
                    .ok();
            })
            .detach();
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
        if let Some(cloud) = study
            .cloud
            .as_ref()
            .filter(|cloud| !copy && !cloud.account.eq_ignore_ascii_case(&account))
        {
            let cloud_account = &cloud.account;
            window.push_notification(
                Notification::warning(format!(
                    "Switch to @{cloud_account} to upload changes to this study"
                )),
                cx,
            );
            return;
        }
        self.uploading = Some(id.clone());
        cx.notify();
        // The board's last edits first, when it shows this study.
        let flushed = kchess_board::analysis::save_study(window, cx);
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
                    .ok_or("The study is gone")?;
                let pgn = document_pgn(&study);
                let uploaded: Vec<String> = study
                    .chapters
                    .iter()
                    .map(|chapter| chapter.id.clone())
                    .collect();
                let linked = study.cloud.as_ref().filter(|cloud| {
                    !copy
                        && !cloud.structure_changed
                        && cloud.account.eq_ignore_ascii_case(&account)
                });
                let reconnect = || format!("Connect @{account} again to allow study access.");
                let (remote, synced) = if let Some(cloud) = linked {
                    let remote = cloud.id.clone();
                    let synced = core
                        .request(calls::SyncLichessStudy {
                            request: StudySyncRequest {
                                account: account.clone(),
                                study_id: remote.clone(),
                                baseline: cloud.downloaded_pgn.clone(),
                                pgn,
                            },
                        })
                        .await
                        .map_err(|cause| cause.message)?;
                    match synced {
                        Reply::Done(chapters) => (remote, Some(chapters)),
                        Reply::NeedsReconnect => return Err(reconnect()),
                    }
                } else {
                    let name: String = study.name.chars().take(100).collect();
                    let exported = core
                        .request(calls::ExportToLichessStudy {
                            account: Username(account.clone()),
                            study_id: StudyId::default(),
                            name: ChapterName(name),
                            pgn: PgnText(pgn),
                        })
                        .await
                        .map_err(|cause| cause.message)?;
                    match exported {
                        Reply::Done(StudyExport { id }) => (id, None),
                        Reply::NeedsReconnect => return Err(reconnect()),
                    }
                };
                let chapters = match synced {
                    Some(chapters) => chapters,
                    None => match core
                        .request(calls::LichessStudyChapters {
                            account: Username(account.clone()),
                            id: LichessId(remote.clone()),
                        })
                        .await
                        .map_err(|cause| cause.message)?
                    {
                        Reply::Done(chapters) => chapters,
                        Reply::NeedsReconnect => {
                            return Err("Uploaded, but study access needs reconnection".into());
                        }
                    },
                };
                let baseline = chapters
                    .iter()
                    .map(|chapter| chapter.pgn.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let marked = this
                    .update(cx, |_, cx| {
                        library.read(cx).command(StudyCommand::MarkCloud {
                            id: id.clone(),
                            account: account.clone(),
                            remote_id: remote.clone(),
                            baseline: baseline.clone(),
                            chapters: uploaded.clone(),
                        })
                    })
                    .map_err(|_| "The page closed".to_string())?;
                let result = marked.await?;
                this.update(cx, |_, cx| {
                    library.update(cx, |library, cx| {
                        library.apply(&result);
                        cx.notify();
                    })
                })
                .ok();
                Ok(())
            }
            .await;
            this.update_in(cx, |this, window, cx| {
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
            })
            .ok();
        })
        .detach();
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
                    .tooltip_with_action("New study", &kchess_nav::page::New, None)
                    .on_click(|_, window, cx| Self::create(window, cx)),
            )
    }

    fn menu(&self, study: &Study) -> impl IntoElement {
        let id = study.id.clone();
        let name = study.name.clone();
        let cloud = study.cloud.is_some();
        let can_copy = study
            .cloud
            .as_ref()
            .is_some_and(|cloud| !cloud.structure_changed);
        let busy = self.uploading.is_some();
        let study = study.clone();
        Button::new(SharedString::from(format!("study-menu-{id}")))
            .ghost()
            .small()
            .icon(icon("ellipsis-vertical"))
            .accessibility_label(format!("Actions for {name}"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let (open, drill, rename, duplicate, copy, save, upload, remove) = (
                    id.clone(),
                    id.clone(),
                    study.clone(),
                    id.clone(),
                    study.clone(),
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
                                StudyCommand::Duplicate {
                                    id: duplicate.clone(),
                                },
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
                    )
                    .item(
                        PopupMenuItem::new("Save PGN…")
                            .icon(icon("download"))
                            .on_click(move |_, window, cx| Self::save_pgn(&save, window, cx)),
                    );
                let menu = if can_copy {
                    menu.item(
                        PopupMenuItem::new("Upload a new cloud copy")
                            .icon(icon("cloud-upload"))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                if let Some(page) = kchess_nav::shell::view::<StudiesPage>(cx) {
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
                    .on_click(move |_, window, cx| Self::remove(remove.clone(), window, cx)),
                )
            })
    }

    /// The rows' heights as measured (`ZERO` for a row not yet made), and the grid's height.
    #[cfg(any(test, feature = "test-support"))]
    pub fn grid_layout(&self) -> (Vec<Pixels>, Pixels) {
        (
            self.row_heights.borrow().heights.clone(),
            self.grid.bounds().size.height,
        )
    }

    /// Scroll the page as only the page itself would, with no new frame asked for (a scroll by
    /// the wheel asks for one).
    #[cfg(any(test, feature = "test-support"))]
    pub fn scroll_to_top(&self) {
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// The PGNs whose summaries are kept, in order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn kept_summaries(&self) -> Vec<String> {
        let mut kept: Vec<String> = self.summaries.borrow().keys().cloned().collect();
        kept.sort();
        kept
    }

    /// A study's summary, kept while the study is listed.
    fn summary(&self, pgn: &str) -> Option<StudySummary> {
        if let Some(kept) = self.summaries.borrow().get(pgn) {
            return kept.clone();
        }
        let made = summary(pgn);
        self.summaries
            .borrow_mut()
            .insert(pgn.to_string(), made.clone());
        made
    }

    fn render_device(
        &mut self,
        width: Pixels,
        window: &Window,
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
                        widgets::card(palette).child(
                            widgets::empty_state(
                                icon("library-big"),
                                "No studies yet",
                                None,
                                palette,
                            )
                            .child(
                                h_flex()
                                    .justify_center()
                                    .gap(px(8.))
                                    .mt(px(8.))
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
                        ),
                    )
                })
                .into_any_element();
        }
        let query = self.search.read(cx).value().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let found: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        let by_name = crate::prefs::get::<String>(cx, SORT, "recent".into()) == "name";
        // Summaries of studies no longer in the library (edited, removed) are let go.
        let listed: std::collections::HashSet<&str> =
            items.iter().map(|study| study.pgn.as_str()).collect();
        self.summaries
            .borrow_mut()
            .retain(|pgn, _| listed.contains(pgn.as_str()));
        let mut studies: Vec<Study> = items
            .iter()
            .filter(|study| {
                let summary = self.summary(&study.pgn);
                let text = format!(
                    "{} {} {}",
                    study.name,
                    summary
                        .as_ref()
                        .map_or("", |summary| summary.players.as_str()),
                    summary
                        .as_ref()
                        .map_or("", |summary| summary.event.as_str())
                )
                .to_lowercase();
                words.iter().all(|word| text.contains(word))
            })
            .cloned()
            .collect();
        if by_name {
            studies.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        } else {
            studies.sort_by(|a, b| b.updated_at_ms().total_cmp(&a.updated_at_ms()));
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
                    "No study matches “{}”",
                    self.search.read(cx).value()
                ))
                .into_any_element()
        } else {
            self.render_cards(studies, found, width, window, cx)
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
            self.render_lichess(width, cx, &palette)
        } else {
            self.render_device(width, window, cx, &palette)
        };
        div()
            .id("studies")
            .size_full()
            .track_focus(&self.focus)
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
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
                                        page.update(cx, |page, cx| {
                                            page.load_lichess(false, window, cx)
                                        })
                                        .ok();
                                    }
                                    window.refresh();
                                }
                            },
                            false,
                            &palette,
                        )))
                    })
                    // Under the tabs, as Puzzles has them: above, the actions shown on one tab and
                    // not the other moved the tabs 52 px under the pointer that switched them.
                    .when(count > 0 || tab == "lichess", |page| {
                        page.child(
                            h_flex()
                                .justify_end()
                                .mb(px(20.))
                                .child(Self::header_actions()),
                        )
                    })
                    .child(div().relative().child(self.area.probe(cx)).child(body)),
            )
    }
}

/// The Studies page's header actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    New,
    Import,
}

impl kchess_nav::page::PageAction for Action {
    fn info(&self) -> kchess_nav::page::ActionInfo {
        use kchess_nav::page::{ActionInfo, ActionRole};
        match self {
            Action::New => ActionInfo::new("studies:new", "New study", "plus", "create chapter")
                .role(ActionRole::New),
            Action::Import => ActionInfo::new(
                "studies:import",
                "Import PGN",
                "import",
                "paste games study",
            ),
        }
    }
}

impl PageView for StudiesPage {
    const INFO: kchess_nav::page::PageInfo = kchess_nav::page::PageInfo {
        page: kchess_nav::page::Page::Studies,
        label: "Studies",
        short: "Studies",
        icon: "library-big",
        group: Some(kchess_nav::page::NavGroup::Review),
        keywords: "pgn chapters notes",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        StudiesPage::new(window, cx)
    }

    fn shown(this: &Entity<Self>, _again: bool, window: &mut Window, cx: &mut App) {
        this.update(cx, |studies, cx| studies.shown(window, cx));
    }

    fn search_fields(&self) -> Vec<Entity<InputState>> {
        vec![self.search.clone()]
    }

    fn actions(&self, _: &App) -> Vec<Action> {
        vec![Action::New, Action::Import]
    }

    fn act(_this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        match action {
            Action::New => StudiesPage::create(window, cx),
            Action::Import => StudiesPage::open_import(window, cx),
        }
    }
}

/// Studies at its Lichess tab: the connected account's own studies.
pub fn open_lichess(window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<StudiesPage>(window, cx, |studies, window, cx| {
        studies.show_lichess(window, cx)
    });
}

/// Ask for a name, and send the study command `command` makes of it: the dialog closes once the
/// library shows the answer, and shows a refusal itself.
pub fn ask_name(
    title: &'static str,
    current: &str,
    confirm: &'static str,
    window: &mut Window,
    cx: &mut App,
    command: impl Fn(String) -> StudyCommand + 'static,
) {
    let library = cx.global::<StudyHandle>().0.clone();
    NameDialog::open(
        title,
        current,
        confirm,
        window,
        cx,
        move |name, dialog, window, cx| {
            let answer = library.read(cx).command(command(name));
            let library = library.clone();
            window
                .spawn(cx, async move |cx| {
                    let result = answer.await;
                    cx.update(|window, cx| match result {
                        Ok(result) => {
                            library.update(cx, |library, cx| {
                                library.apply(&result);
                                cx.notify();
                            });
                            window.close_dialog(cx);
                        }
                        Err(cause) => dialog.update(cx, |dialog, cx| dialog.fail(cause, cx)),
                    })
                    .ok();
                })
                .detach();
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{
        import_chapters, import_name, natural_cmp, resolved_heights, rows_in_view, view_span,
    };

    /// Other pages open Studies at its Lichess tab, and a study's Drill opens Practice on it;
    /// each asks the shell to open its page.
    #[gpui_kit::test]
    fn studies_open_and_hand_a_study_to_practice(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = kchess_board::testing::Started::new(cx);
        let shell = app.with_window(|window, cx| {
            crate::studies::init(&Default::default(), cx);
            let page = cx.new(|cx| super::StudiesPage::new(window, cx));
            let shell = std::rc::Rc::new(kchess_nav::testing::OnlyPage::of(page));
            cx.set_global(kchess_nav::shell::Navigator(shell.clone()));
            shell
        });
        let tab = app.with_window(|window, cx| {
            super::open_lichess(window, cx);
            crate::prefs::get::<String>(cx, super::TAB, String::new())
        });
        assert_eq!(tab, "lichess");
        app.with_window(|window, cx| super::StudiesPage::drill("s1".into(), window, cx));
        assert_eq!(*shell.opened.borrow(), ["Studies", "Practice"]);
    }
    use gpui_kit::{Bounds, point, px, size};
    use std::cmp::Ordering;

    /// Only the rows reaching into view are made; the spaces left above and below keep the grid
    /// as tall as it is, row gaps included, so scrolling lands where it did.
    #[test]
    fn only_the_rows_in_view_are_made() {
        let heights = [px(100.); 10];
        let gap = px(10.);
        // Rows start at 0, 110, 220, … and end 100 later; the grid is 1090 tall.
        let (rows, above, below) = rows_in_view(&heights, gap, px(0.), px(250.));
        assert_eq!((rows, above, below), (0..3, px(0.), px(770.)));
        // A row starting where the view ends is in it.
        assert_eq!(rows_in_view(&heights, gap, px(0.), px(330.)).0, 0..4);
        let (rows, above, below) = rows_in_view(&heights, gap, px(300.), px(500.));
        assert_eq!((rows, above, below), (2..5, px(210.), px(550.)));
        // Above + made rows (with their gaps) + below is the whole grid.
        let made = px(100.) * 3. + gap * 3.;
        assert_eq!(above + made + below, px(1090.));
        // A row ending where the view starts is still in it.
        let (rows, above, below) = rows_in_view(&heights, gap, px(980.), px(2000.));
        assert_eq!((rows, above, below), (8..10, px(870.), px(0.)));
        // Scrolled past the grid, and an empty grid.
        let (rows, above, _) = rows_in_view(&heights, gap, px(5000.), px(6000.));
        assert_eq!((rows, above), (0..0, px(1090.)));
        assert_eq!(rows_in_view(&[], gap, px(0.), px(100.)).0, 0..0);
        // A row taller than the others counts as tall as it is.
        let mixed = [px(100.), px(300.), px(100.)];
        assert_eq!(
            rows_in_view(&mixed, gap, px(450.), px(600.)),
            (2..3, px(410.), px(0.))
        );
    }

    /// Rows measured at one card width are let go at another: a row made at the first frame's
    /// width and then scrolled out of view kept that height, and the grid came out 18.5 px short.
    #[test]
    fn row_heights_are_kept_for_one_card_width() {
        let mut rows = super::RowHeights::default();
        rows.fit(px(210.), 3);
        assert!(rows.record(1, px(387.)));
        assert!(!rows.record(1, px(387.)), "the same height is nothing new");
        assert!(!rows.record(5, px(10.)), "no such row");
        rows.fit(px(210.), 4);
        assert_eq!(rows.heights, [px(0.), px(387.), px(0.), px(0.)]);
        rows.fit(px(240.), 4);
        assert_eq!(rows.heights, [px(0.); 4]);
    }

    /// A row not yet laid out is taken to be as tall as the first one measured, and every row as
    /// tall as the estimate before any is.
    #[test]
    fn rows_not_laid_out_take_the_measured_height() {
        let zero = px(0.);
        assert_eq!(
            resolved_heights(&[zero, px(320.), px(330.), zero], px(100.)),
            [px(320.), px(320.), px(330.), px(320.)]
        );
        assert_eq!(
            resolved_heights(&[zero, zero], px(100.)),
            [px(100.), px(100.)]
        );
        // A card's estimated row is near what one measures (a 210 px card: 361 px).
        let estimate = super::estimated_row(px(210.));
        assert!(estimate > px(330.) && estimate < px(400.), "{estimate:?}");
    }

    /// The span to make is what the viewport shows of the grid, measured from the grid's top, and
    /// as much again above and below (at least 200 px); before the first layout, the window.
    #[test]
    fn the_span_made_is_the_view_and_a_margin() {
        let at = |top: f32, height: f32| Bounds {
            origin: point(px(0.), px(top)),
            size: size(px(800.), px(height)),
        };
        // The viewport shows 100..700 of the window; the grid starts 1000 above it (scrolled).
        assert_eq!(
            view_span(at(100., 600.), at(-900., 5000.), px(800.)),
            (px(400.), px(2200.))
        );
        // A short viewport keeps a 200 px margin.
        assert_eq!(
            view_span(at(0., 50.), at(0., 5000.), px(800.)),
            (px(-200.), px(250.))
        );
        // Not laid out yet: the window from the top, and as much again.
        let none = Bounds::default();
        assert_eq!(
            view_span(none, at(0., 100.), px(800.)),
            (px(-800.), px(1600.))
        );
        assert_eq!(
            view_span(at(0., 600.), none, px(800.)),
            (px(-800.), px(1600.))
        );
    }

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
        assert!(import_chapters("").is_err());
        // A study holds at most 64 chapters.
        assert!(import_chapters(&"[Event \"?\"]\n\n1. e4 *\n\n".repeat(65)).is_err());
        // An Atomic chapter is kept in Atomic; a variant KChess does not play is refused.
        assert!(import_chapters("[Variant \"Atomic\"]\n\n1. e4 d5 2. exd5 *").is_ok());
        assert_eq!(
            import_chapters("[Variant \"Bughouse\"]\n\n1. e4 d5 2. exd5 *"),
            Err(super::VARIANT_IMPORT)
        );
        assert!(import_chapters("[Variant \"Chess960\"]\n[FEN \"bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1\"]\n\n1. e4 *").is_ok());
    }
}
