//! Studies: the study library on this computer. Browse, search and open saved
//! studies on the analysis board, start or import one, and rename, copy, drill, upload or delete
//! them. Lichess's own studies are a tab of their own when an account is connected.

use kchess_contracts::calls;
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
use kchess_domain::records::studies::StudySummary;
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

/// What a study's card shows of it: its final position, players and event; none for a PGN that
/// cannot be imported.
fn summary(pgn: &str) -> Option<StudySummary> {
    kchess_domain::records::studies::summarize_study(pgn)
}

/// The rows' heights as laid out at one width of card: at another, a card's text wraps
/// otherwise, so a row measured before (one made at the first frame's width, then scrolled out of
/// view) no longer stands for its height.
#[derive(Default)]
struct RowHeights {
    card_width: Pixels,
    heights: Vec<Pixels>,
}

impl RowHeights {
    /// `rows` rows of cards `card_width` wide: measurements at another width are let go.
    fn fit(&mut self, card_width: Pixels, rows: usize) {
        if self.card_width != card_width {
            self.card_width = card_width;
            self.heights.clear();
        }
        self.heights.resize(rows, Pixels::ZERO);
    }

    /// A row laid out at `height`; whether that is new.
    fn record(&mut self, row: usize, height: Pixels) -> bool {
        match self.heights.get_mut(row) {
            Some(kept) if *kept != height => {
                *kept = height;
                true
            }
            _ => false,
        }
    }
}

/// A row of cards' height before any is laid out: the square picture and the text under it.
pub(crate) fn estimated_row(card_width: Pixels) -> Pixels {
    card_width + px(150.)
}

/// Each row's height as measured, or as tall as the first measured row for one not yet laid out
/// (`ZERO`); `estimate` before any is.
fn resolved_heights(measured: &[Pixels], estimate: Pixels) -> Vec<Pixels> {
    let usual = measured
        .iter()
        .copied()
        .find(|height| *height > Pixels::ZERO)
        .unwrap_or(estimate);
    measured
        .iter()
        .map(|height| {
            if *height > Pixels::ZERO {
                *height
            } else {
                usual
            }
        })
        .collect()
}

/// What of the grid to make, from its top: what the page's viewport shows of it and as much again
/// (at least 200 px) above and below, so a scroll finds its cards made. Before the first layout,
/// the window's height from the top.
fn view_span(viewport: Bounds<Pixels>, grid: Bounds<Pixels>, window: Pixels) -> (Pixels, Pixels) {
    let (top, bottom) = if viewport.size.height > Pixels::ZERO && grid.size.height > Pixels::ZERO {
        (viewport.top() - grid.top(), viewport.bottom() - grid.top())
    } else {
        (Pixels::ZERO, window)
    };
    let margin = (bottom - top).max(px(200.));
    (top - margin, bottom + margin)
}

/// The rows of a grid to make: those reaching into `top..bottom` (from the grid's top), with the
/// heights the rows left out take above and below them. Rows are `gap` apart.
fn rows_in_view(
    heights: &[Pixels],
    gap: Pixels,
    top: Pixels,
    bottom: Pixels,
) -> (std::ops::Range<usize>, Pixels, Pixels) {
    let (mut first, mut last) = (heights.len(), 0);
    let (mut above, mut below) = (Pixels::ZERO, Pixels::ZERO);
    let mut y = Pixels::ZERO;
    for (row, height) in heights.iter().enumerate() {
        let space = if row > 0 { gap } else { Pixels::ZERO };
        let (start, end) = (y + space, y + space + *height);
        if end < top {
            above = end;
        } else if start <= bottom {
            first = first.min(row);
            last = row + 1;
        } else {
            below += space + *height;
        }
        y = end;
    }
    if first >= last {
        // Nothing reaches into view (an empty grid): every row is space above.
        return (0..0, y, Pixels::ZERO);
    }
    (first..last, above, below)
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

/// Why an import was refused.
const INVALID_IMPORT: &str = "Paste valid PGN with up to 64 chapters";
const VARIANT_IMPORT: &str = "A chapter is in a variant KChess doesn't play";

/// An import's chapters (`pgnGames`): each game named by its `ChapterName`, or numbered.
fn import_chapters(text: &str) -> Result<Vec<(String, String)>, &'static str> {
    let chapters: Vec<(String, String)> = kchess_domain::position::pgn_games(text)
        .into_iter()
        .enumerate()
        .map(|(index, game)| {
            let name = game
                .headers
                .iter()
                .find(|(name, _)| name == "ChapterName")
                .map_or_else(
                    || format!("Chapter {}", index + 1),
                    |(_, value)| value.clone(),
                );
            (name, game.pgn)
        })
        .collect();
    // A game read under rules other than its own would show positions it never had.
    if chapters
        .iter()
        .any(|(_, pgn)| !crate::game::pgn_analysable(pgn))
    {
        return Err(VARIANT_IMPORT);
    }
    let valid = !chapters.is_empty()
        && chapters.len() <= MAX_CHAPTERS
        && chapters
            .iter()
            .all(|(_, pgn)| TreeNode::from_pgn(pgn).is_some());
    if valid {
        Ok(chapters)
    } else {
        Err(INVALID_IMPORT)
    }
}

/// An imported study's name when none is given: its players, else its event.
fn import_name(first: &str) -> Option<String> {
    let headers = TreeNode::from_pgn(first)?
        .headers
        .take()
        .unwrap_or_default();
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
    /// Its search field, for Find (`shell::find_on_page`).
    pub(crate) fn search_fields(&self) -> Vec<Entity<InputState>> {
        vec![self.search.clone()]
    }

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
            scroll: ScrollHandle::new(),
            grid: Measured::default(),
            row_heights: Default::default(),
            summaries: Default::default(),
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

    /// Show the Lichess tab, as its pill does.
    pub fn show_lichess(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, TAB, "lichess");
        self.load_lichess(false, cx);
        window.refresh();
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
        crate::pages::analysis::when_read(window, cx, move |analysis, window, cx| {
            if analysis.read(cx).study_id() == id {
                crate::shell::open(Page::Analysis, window, cx);
                return;
            }
            crate::pages::analysis::replace_read(
                analysis,
                window,
                cx,
                move |analysis, window, cx| match analysis.open_study(&id, None, cx) {
                    Ok(()) => true,
                    Err(cause) => {
                        window.push_notification(Notification::error(cause), cx);
                        false
                    }
                },
            );
        });
    }

    /// `New study`: an empty board, kept as a study at once.
    pub(crate) fn create(window: &mut Window, cx: &mut App) {
        let name = Self::library(cx).read(cx).fresh_name("Untitled study");
        crate::pages::analysis::replace(window, cx, move |analysis, window, cx| {
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
        crate::pages::practice::repertoire::train_study(id, window, cx);
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

    /// Save a study's PGN, every chapter, as a file where the player picks.
    fn save_pgn(study: &Value, window: &mut Window, cx: &mut App) {
        let name: String = study["name"]
            .as_str()
            .unwrap_or("study")
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
                let _ = cx.update(|window, cx| window.push_notification(note, cx));
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
        let cloud = study["cloud"].clone();
        let cloud_account = cloud["account"].as_str().unwrap_or_default();
        if !copy && !cloud.is_null() && !cloud_account.eq_ignore_ascii_case(&account) {
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
        let flushed = crate::pages::analysis::board(window, cx)
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
                    .ok_or("The study is gone")?;
                let pgn = document_pgn(&study);
                let cloud = study["cloud"].clone();
                let linked = !copy
                    && !cloud.is_null()
                    && cloud["structureChanged"] != json!(true)
                    && cloud["account"]
                        .as_str()
                        .is_some_and(|owner| owner.eq_ignore_ascii_case(&account));
                let result = if linked {
                    core.request(calls::SyncLichessStudy {
                        request: json!({
                            "account": account,
                            "studyId": cloud["id"],
                            "baseline": cloud["downloadedPgn"],
                            "pgn": pgn,
                        }),
                    })
                    .await
                } else {
                    let name: String = study["name"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(100)
                        .collect();
                    core.request(calls::ExportToLichessStudy {
                        account: json!(account),
                        study_id: json!(""),
                        name: json!(name),
                        pgn: json!(pgn),
                    })
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
                    core.request(calls::LichessStudyChapters {
                        account: json!(account),
                        id: json!(remote),
                    })
                    .await
                    .map_err(|cause| cause.message)?
                };
                if chapters.get("needsReconnect").is_some() {
                    return Err("Uploaded, but study access needs reconnection".into());
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
                    .map_err(|_| "The page closed".to_string())?;
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

    pub(crate) fn open_import(window: &mut Window, cx: &mut App) {
        Self::open_import_with(None, window, cx);
    }

    /// The import dialog holding `pgn` already, as a file of several games dropped on the window
    /// offers it.
    pub(crate) fn open_import_with(pgn: Option<String>, window: &mut Window, cx: &mut App) {
        let import = cx.new(|cx| ImportStudy::new(window, cx));
        if let Some(pgn) = pgn {
            import.update(cx, |import, cx| {
                import
                    .text
                    .update(cx, |text, cx| text.set_value(pgn, window, cx));
            });
        }
        crate::widgets::open_dialog(window, cx, move |dialog, _, _| {
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
                    .tooltip_with_action("New study", &crate::menu::New, None)
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
                                if let Some(page) = crate::shell::view::<StudiesPage>(cx) {
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

    /// The rows' heights as measured (`ZERO` for a row not yet made), and the grid's height.
    #[cfg(test)]
    pub(crate) fn grid_layout(&self) -> (Vec<Pixels>, Pixels) {
        (
            self.row_heights.borrow().heights.clone(),
            self.grid.bounds().size.height,
        )
    }

    /// Scroll the page as only the page itself would, with no new frame asked for (a scroll by
    /// the wheel asks for one).
    #[cfg(test)]
    pub(crate) fn scroll_to_top(&self) {
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// The PGNs whose summaries are kept, in order.
    #[cfg(test)]
    pub(crate) fn kept_summaries(&self) -> Vec<String> {
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

    fn render_cards(
        &mut self,
        studies: Vec<Value>,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let now = now();
        // Pictures of studies no longer listed (edited, removed, filtered out) are let go; those
        // of listed studies scrolled out of view are kept for when they come back.
        let listed: std::collections::HashSet<String> = studies
            .iter()
            .filter_map(|study| {
                self.summary(study["pgn"].as_str().unwrap_or_default())
                    .map(|summary| summary.fen)
            })
            .collect();
        self.previews.retain(|fen, _| listed.contains(fen));
        let rows = studies.len().div_ceil(count);
        let heights = {
            let mut measured = self.row_heights.borrow_mut();
            measured.fit(card_width, rows);
            resolved_heights(&measured.heights, estimated_row(card_width))
        };
        let (top, bottom) = view_span(
            self.scroll.bounds(),
            self.grid.bounds(),
            window.viewport_size().height,
        );
        let (shown, above, below) = rows_in_view(&heights, CARD_GAP, top, bottom);
        let mut grid_rows: Vec<AnyElement> = Vec::new();
        for row in shown.clone() {
            let cards: Vec<AnyElement> = studies
                .iter()
                .skip(row * count)
                .take(count)
                .map(|study| {
                    let id = study["id"].as_str().unwrap_or_default().to_string();
                    let name = study["name"].as_str().unwrap_or_default().to_string();
                    let summary = self.summary(study["pgn"].as_str().unwrap_or_default());
                    let board = summary
                        .as_ref()
                        .map(|summary| self.preview(&summary.fen, cx).into());
                    let event = summary
                        .as_ref()
                        .map(|summary| summary.event.as_str())
                        .filter(|event| *event != name);
                    let players = summary.as_ref().map(|summary| summary.players.as_str());
                    let description = [players, event]
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
                        width: card_width,
                        on_open: std::rc::Rc::new(move |window, cx| {
                            Self::open(open_id.clone(), window, cx)
                        }),
                    }
                    .into_any_element()
                })
                .collect();
            let heights = std::rc::Rc::clone(&self.row_heights);
            let entity = cx.entity().downgrade();
            // The row's height as laid out, for the rows' spacing while it is out of view.
            let probe = canvas(
                move |area, window, _| {
                    if heights.borrow_mut().record(row, area.size.height) {
                        let entity = entity.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = entity.update(cx, |_, cx| cx.notify());
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full();
            grid_rows.push(
                h_flex()
                    .relative()
                    .items_stretch()
                    .gap(CARD_GAP)
                    .when(row > 0, |line| line.mt(CARD_GAP))
                    .child(probe)
                    .children(cards)
                    .into_any_element(),
            );
        }
        v_flex()
            .relative()
            .child(self.grid.probe(cx))
            .child(div().h(above))
            .children(grid_rows)
            .child(div().h(below))
            .into_any_element()
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
        let by_name = crate::prefs::get::<String>(cx, SORT, "recent".into()) == "name";
        // Summaries of studies no longer in the library (edited, removed) are let go.
        let listed: std::collections::HashSet<&str> = items
            .iter()
            .filter_map(|study| study["pgn"].as_str())
            .collect();
        self.summaries
            .borrow_mut()
            .retain(|pgn, _| listed.contains(pgn.as_str()));
        let mut studies: Vec<Value> = items
            .iter()
            .filter(|study| {
                let summary = self.summary(study["pgn"].as_str().unwrap_or_default());
                let text = format!(
                    "{} {} {}",
                    study["name"].as_str().unwrap_or_default(),
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
                    "No study matches “{}”",
                    self.search.read(cx).value()
                ))
                .into_any_element()
        } else {
            self.render_cards(studies, width, window, cx)
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
        let chapters = match import_chapters(&text) {
            Ok(chapters) => chapters,
            Err(problem) => {
                self.error = Some(problem.into());
                cx.notify();
                return;
            }
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
                    .child("Paste PGN chapters"),
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
    use super::{
        import_chapters, import_name, natural_cmp, resolved_heights, rows_in_view, view_span,
    };
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

/// The Studies page's header actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    New,
    Import,
}

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::{ActionInfo, ActionRole};
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

impl crate::pages::PageView for StudiesPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Studies,
        label: "Studies",
        short: "Studies",
        icon: "library-big",
        group: Some(crate::pages::NavGroup::Review),
        keywords: "pgn chapters notes",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        StudiesPage::new(window, cx)
    }

    fn shown(this: &Entity<Self>, _again: bool, _window: &mut Window, cx: &mut App) {
        this.update(cx, |studies, cx| studies.shown(cx));
    }

    fn search_fields(&self) -> Vec<Entity<InputState>> {
        StudiesPage::search_fields(self)
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
    crate::shell::show::<StudiesPage>(window, cx, |studies, window, cx| {
        studies.show_lichess(window, cx)
    });
}
