//! Studies → Lichess: the studies on a connected
//! account, opened (kept offline first) or kept offline to open later, and the analysis board sent
//! to one of them or to a new private study.

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{CARD_GAP, CARD_MIN, StudiesPage, summary};
use crate::assets::icon;
use crate::core::AppCore;
use crate::studies::{StudyHandle, now, time_ago};
use crate::theme::Palette;
use crate::widgets::{self, Choice};

/// A list fetched this recently is shown without asking again; a failure waits this long.
const FRESH_FOR: f64 = 5. * 60_000.;
const RETRY_AFTER: f64 = 60_000.;
const NEW_STUDY: &str = "new-study";

/// One account's list (`Entry`).
#[derive(Default, Clone)]
struct Entry {
    items: Option<Vec<Value>>,
    fetched_at: f64,
    attempted_at: f64,
    loading: bool,
    error: String,
    needs_reconnect: bool,
}

/// The Lichess tab's state on the Studies page.
pub struct LichessTab {
    account: String,
    entries: HashMap<String, Entry>,
    busy: bool,
    message: String,
    target: Entity<SelectState<Vec<Choice>>>,
    account_select: Entity<SelectState<Vec<Choice>>>,
}

fn needs_reconnect(result: &Value) -> bool {
    result.get("needsReconnect").is_some()
}

pub fn new_tab(
    window: &mut Window,
    cx: &mut Context<StudiesPage>,
) -> (LichessTab, Vec<Subscription>) {
    let target = cx.new(|cx| {
        SelectState::new(
            vec![Choice::new(NEW_STUDY, "A new private study")],
            None,
            window,
            cx,
        )
    });
    target.update(cx, |state, cx| {
        state.set_selected_value(&SharedString::from(NEW_STUDY), window, cx)
    });
    let account_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
    let subscriptions = vec![cx.subscribe(
        &account_select,
        |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
            if let SelectEvent::Confirm(Some(value)) = event {
                this.lichess.account = value.to_string();
                this.lichess.message.clear();
                this.load_lichess(false, cx);
            }
        },
    )];
    (
        LichessTab {
            account: String::new(),
            entries: HashMap::new(),
            busy: false,
            message: String::new(),
            target,
            account_select,
        },
        subscriptions,
    )
}

impl StudiesPage {
    fn entry(&self) -> Option<&Entry> {
        self.lichess
            .entries
            .get(&self.lichess.account.to_lowercase())
    }

    /// `refresh`: the account's studies, unless fetched (or failed) a moment ago.
    pub(super) fn load_lichess(&mut self, force: bool, cx: &mut Context<Self>) {
        let account = self.lichess.account.clone();
        if account.is_empty() {
            return;
        }
        let key = account.to_lowercase();
        let entry = self.lichess.entries.entry(key.clone()).or_default();
        let at = now();
        if entry.loading
            || (!force
                && (entry.needs_reconnect
                    || (entry.items.is_some() && at - entry.fetched_at < FRESH_FOR)
                    || (entry.attempted_at > 0. && at - entry.attempted_at < RETRY_AFTER)))
        {
            return;
        }
        entry.loading = true;
        entry.error.clear();
        entry.attempted_at = at;
        cx.notify();
        let studies = cx
            .global::<AppCore>()
            .call("lichessStudies", vec![json!(account)]);
        cx.spawn(async move |this, cx| {
            let result = studies.await;
            let _ = this.update(cx, |this, cx| {
                let Some(entry) = this.lichess.entries.get_mut(&key) else {
                    return;
                };
                entry.loading = false;
                match result {
                    Ok(result) if needs_reconnect(&result) => entry.needs_reconnect = true,
                    Ok(result) => {
                        entry.items = Some(result.as_array().cloned().unwrap_or_default());
                        entry.fetched_at = now();
                        entry.needs_reconnect = false;
                    }
                    Err(cause) => entry.error = cause.message,
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The account the tab shows follows the one playing online, as Electron's does.
    pub(super) fn sync_lichess_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let accounts = crate::accounts::handle(cx);
        let (connected, active) = {
            let accounts = accounts.read(cx);
            (accounts.connected(), accounts.active(cx))
        };
        let items: Vec<Choice> = connected
            .iter()
            .map(|name| Choice::new(name.clone(), format!("@{name}")))
            .collect();
        self.lichess
            .account_select
            .update(cx, |state, cx| state.set_items(items, window, cx));
        let keep = connected
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&self.lichess.account));
        if !keep {
            self.lichess.account = active.unwrap_or_default();
        }
        let chosen = self.lichess.account.clone();
        self.lichess.account_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(chosen), window, cx)
        });
        // Accounts that are gone take their lists with them.
        let kept: Vec<String> = connected.iter().map(|name| name.to_lowercase()).collect();
        self.lichess.entries.retain(|key, _| kept.contains(key));
        let mut targets = vec![Choice::new(NEW_STUDY, "A new private study")];
        if let Some(items) = self.entry().and_then(|entry| entry.items.clone()) {
            targets.extend(items.iter().map(|study| {
                Choice::new(
                    study["id"].as_str().unwrap_or_default().to_string(),
                    study["name"].as_str().unwrap_or_default().to_string(),
                )
            }));
        }
        self.lichess
            .target
            .update(cx, |state, cx| state.set_items(targets, window, cx));
    }

    /// The offline copy of a cloud study on this account.
    fn offline_copy(&self, id: &str, cx: &App) -> Option<Value> {
        let account = self.lichess.account.to_lowercase();
        cx.global::<StudyHandle>()
            .0
            .read(cx)
            .items()
            .iter()
            .find(|study| {
                study["cloud"]["id"] == json!(id)
                    && study["cloud"]["account"]
                        .as_str()
                        .is_some_and(|owner| owner.to_lowercase() == account)
            })
            .cloned()
    }

    /// Keep a cloud study on this computer (`library.offline`); its id in the library, and whether
    /// offline edits were kept apart from the cloud version.
    fn keep_offline(
        account: String,
        item: Value,
        window: &mut Window,
        cx: &mut App,
        then: impl FnOnce(String, bool, &mut Window, &mut App) + 'static,
        fail: impl FnOnce(String, &mut Window, &mut App) + 'static,
    ) {
        let id = item["id"].as_str().unwrap_or_default().to_string();
        let chapters = cx
            .global::<AppCore>()
            .call("lichessStudyChapters", vec![json!(account), json!(id)]);
        let library = cx.global::<StudyHandle>().0.clone();
        window
            .spawn(cx, async move |cx| {
                let chapters = match chapters.await {
                    Ok(chapters) if needs_reconnect(&chapters) => {
                        let note = format!(
                            "Lichess needs the study permission: connect @{account} again in Settings → My accounts."
                        );
                        let _ = cx.update(|window, cx| fail(note, window, cx));
                        return;
                    }
                    Ok(chapters) => chapters,
                    Err(cause) => {
                        let _ = cx.update(|window, cx| fail(cause.message, window, cx));
                        return;
                    }
                };
                let chapters: Vec<Value> = chapters
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|chapter| json!({ "name": chapter["name"], "pgn": chapter["pgn"] }))
                    .collect();
                let command = json!({
                    "op": "offline",
                    "account": account,
                    "remote": { "id": id, "name": item["name"] },
                    "chapters": chapters,
                });
                let answer = cx.update(|_, cx| library.read(cx).command(command));
                let Ok(answer) = answer else { return };
                let result = answer.await;
                let _ = cx.update(|window, cx| match result {
                    Ok(result) => {
                        library.update(cx, |library, cx| {
                            library.apply(&result);
                            cx.notify();
                        });
                        match result["id"].as_str() {
                            Some(saved) => then(
                                saved.to_string(),
                                result["conflict"] == json!(true),
                                window,
                                cx,
                            ),
                            None => fail("The study could not be saved offline.".into(), window, cx),
                        }
                    }
                    Err(cause) => fail(cause, window, cx),
                });
            })
            .detach();
    }

    /// `openRemote`: open the offline copy, or keep the study offline first.
    fn open_remote(&mut self, item: Value, window: &mut Window, cx: &mut Context<Self>) {
        let id = item["id"].as_str().unwrap_or_default().to_string();
        if let Some(saved) = self.offline_copy(&id, cx) {
            let saved = saved["id"].as_str().unwrap_or_default().to_string();
            Self::open(saved, window, cx);
            return;
        }
        self.lichess.busy = true;
        self.lichess.message.clear();
        cx.notify();
        let page = cx.entity().downgrade();
        let failed = page.clone();
        Self::keep_offline(
            self.lichess.account.clone(),
            item,
            window,
            cx,
            move |saved, _, window, cx| {
                let _ = page.update(cx, |page, cx| {
                    page.lichess.busy = false;
                    cx.notify();
                });
                Self::open(saved, window, cx);
            },
            move |note, _, cx| {
                let _ = failed.update(cx, |page, cx| {
                    page.lichess.busy = false;
                    page.lichess.message = note;
                    cx.notify();
                });
            },
        );
    }

    /// `download`: keep (or refresh) the offline copy; the study on the board takes it in.
    fn download(&mut self, item: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.lichess.busy = true;
        self.lichess.message.clear();
        cx.notify();
        // The board's own edits reach the library before the cloud copy is merged in.
        let flushed = crate::shell::analysis_board(window, cx)
            .map(|analysis| analysis.update(cx, |analysis, cx| analysis.flush_study(cx)));
        cx.spawn_in(window, async move |this, cx| {
            if let Some(flushed) = flushed {
                let _ = flushed.await;
            }
            let _ = this.update_in(cx, |this, window, cx| this.merge_cloud(item, window, cx));
        })
        .detach();
    }

    /// Keep the cloud copy, then reopen it on the board when the board shows that study.
    fn merge_cloud(&mut self, item: Value, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity().downgrade();
        let failed = page.clone();
        Self::keep_offline(
            self.lichess.account.clone(),
            item,
            window,
            cx,
            move |saved, conflict, window, cx| {
                if let Some(analysis) = crate::shell::analysis_board(window, cx)
                    && analysis.read(cx).study_id() == saved
                {
                    let chapter = analysis.read(cx).chapter_id().to_string();
                    analysis.update(cx, |analysis, cx| {
                        let _ = analysis.open_study(&saved, Some(&chapter), cx);
                    });
                }
                let _ = page.update(cx, |page, cx| {
                    page.lichess.busy = false;
                    page.lichess.message = if conflict {
                        "Your offline edits were kept. The cloud version was downloaded as a separate copy.".into()
                    } else {
                        "Study available offline.".into()
                    };
                    cx.notify();
                });
            },
            move |note, _, cx| {
                let _ = failed.update(cx, |page, cx| {
                    page.lichess.busy = false;
                    page.lichess.message = note;
                    cx.notify();
                });
            },
        );
    }

    /// `send`: the analysis board as a chapter of a study (or a new private one).
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(analysis) = crate::shell::analysis_board(window, cx) else {
            return;
        };
        let (pgn, chapter_name) = {
            let board = analysis.read(cx);
            let study = cx
                .global::<StudyHandle>()
                .0
                .read(cx)
                .study(board.study_id())
                .and_then(|study| study["name"].as_str().map(str::to_string));
            let name = study
                .or_else(|| Some(board.header("Event")).filter(|event| !event.is_empty()))
                .unwrap_or_else(|| "KChess analysis".into());
            (board.pgn(), name.chars().take(100).collect::<String>())
        };
        let account = self.lichess.account.clone();
        let target = self
            .lichess
            .target
            .read(cx)
            .selected_value()
            .map(|value| value.to_string())
            .filter(|value| value != NEW_STUDY)
            .unwrap_or_default();
        self.lichess.busy = true;
        self.lichess.message.clear();
        cx.notify();
        let sent = cx.global::<AppCore>().call(
            "exportToLichessStudy",
            vec![
                json!(account),
                json!(target),
                json!(chapter_name),
                json!(pgn),
            ],
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = sent.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.lichess.busy = false;
                match result {
                    Ok(result) if needs_reconnect(&result) => {
                        this.lichess.message = format!(
                            "Lichess needs the study permission: connect @{account} again in Settings → My accounts."
                        );
                    }
                    Ok(_) => {
                        this.lichess.message = if target.is_empty() {
                            format!("Created a private study with “{chapter_name}”.")
                        } else {
                            format!("Added “{chapter_name}” as a chapter.")
                        };
                        this.load_lichess(true, cx);
                    }
                    Err(cause) => window.push_notification(
                        Notification::error(cause.message).title("Could not send to Lichess"),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_lichess(
        &mut self,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        self.sync_lichess_account(window, cx);
        if self.entry().is_none() {
            self.load_lichess(false, cx);
        }
        let entry = self.entry().cloned().unwrap_or_default();
        let several = crate::accounts::handle(cx).read(cx).connected().len() > 1;
        let status = if !self.lichess.message.is_empty() {
            self.lichess.message.clone()
        } else if !entry.error.is_empty() {
            entry.error.clone()
        } else if entry.needs_reconnect {
            format!(
                "Connect @{} again in Settings → My accounts to allow study access.",
                self.lichess.account
            )
        } else if entry.items.as_ref().is_some_and(Vec::is_empty) {
            "No studies on this account yet.".into()
        } else {
            String::new()
        };
        let toolbar = h_flex()
            .gap(px(8.))
            .when(several, |row| {
                row.child(
                    div()
                        .w(px(180.))
                        .child(Select::new(&self.lichess.account_select).small()),
                )
            })
            .child(
                Button::new("lichess-studies-refresh")
                    .outline()
                    .small()
                    .icon(icon("refresh-cw"))
                    .label("Refresh")
                    .loading(entry.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.load_lichess(true, cx))),
            );
        let items = entry.items.unwrap_or_default();
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let at = now();
        let busy = self.lichess.busy;
        let cards: Vec<AnyElement> = items
            .iter()
            .map(|item| {
                let id = item["id"].as_str().unwrap_or_default().to_string();
                let copy = self.offline_copy(&id, cx);
                let board = copy
                    .as_ref()
                    .and_then(|copy| {
                        summary(copy["pgn"].as_str().unwrap_or_default())["fen"]
                            .as_str()
                            .map(str::to_string)
                    })
                    .map(|fen| self.preview(&fen, cx).into());
                let chapters = copy
                    .as_ref()
                    .and_then(|copy| copy["chapters"].as_array().map(Vec::len))
                    .filter(|count| *count > 0);
                let location = format!(
                    "{}{}",
                    if copy.is_some() {
                        "Offline · Lichess"
                    } else {
                        "Lichess"
                    },
                    chapters
                        .map(|count| format!(
                            " · {count} {}",
                            if count == 1 { "chapter" } else { "chapters" }
                        ))
                        .unwrap_or_default()
                );
                let page = cx.entity().downgrade();
                let (opened, downloaded) = (item.clone(), item.clone());
                widgets::StudyCard {
                    id: format!("lichess-{id}"),
                    name: item["name"].as_str().unwrap_or_default().to_string(),
                    board,
                    location,
                    description: None,
                    edited: item["updatedAt"]
                        .as_f64()
                        .filter(|at| *at > 0.)
                        .map(|updated| format!("Edited {}", time_ago(updated, at))),
                    rows: 3,
                    menu: None,
                    action: Button::new(SharedString::from(format!("keep-{id}")))
                        .outline()
                        .xsmall()
                        .w_full()
                        .icon(icon("download"))
                        .label(if copy.is_some() {
                            "Download latest"
                        } else {
                            "Make available offline"
                        })
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.download(downloaded.clone(), window, cx)
                        }))
                        .into_any_element(),
                    width: card_width,
                    on_open: std::rc::Rc::new(move |window, cx| {
                        let _ = page
                            .update(cx, |page, cx| page.open_remote(opened.clone(), window, cx));
                    }),
                }
                .into_any_element()
            })
            .collect();
        let mut rows = Vec::new();
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            rows.push(
                h_flex()
                    .gap(CARD_GAP)
                    .items_start()
                    .children(cards.by_ref().take(count)),
            );
        }
        let send = widgets::card(palette).child(
            h_flex()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Send the analysis board to"),
                )
                .child(
                    div()
                        .min_w(px(200.))
                        .child(Select::new(&self.lichess.target).small()),
                )
                .child(
                    Button::new("lichess-studies-send")
                        .primary()
                        .small()
                        .icon(icon("upload"))
                        .label("Send")
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                ),
        );
        v_flex()
            .gap(px(16.))
            .child(toolbar)
            .when(!status.is_empty(), |tab| {
                tab.child(div().text_size(px(13.)).child(status))
            })
            .child(v_flex().gap(CARD_GAP).children(rows))
            .child(send)
            .into_any_element()
    }
}
