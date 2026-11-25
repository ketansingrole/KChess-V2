//! Download strong Lichess games: the dialog that asks how strong (both players rated at least
//! a floor) and which month of Lichess's rated games, and says how large that month's download
//! is. The month is read as it downloads and only its strong games are kept, in the floor's own
//! database; the download then runs as the page's import does, its progress on the import card.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_board::widgets;
use kchess_contracts::calls;
use kchess_contracts::check::LichessMonth;
use kchess_contracts::types::{LichessArchive, LichessGames};

use crate::core::AppCore;
use crate::game_db::GameDbHandle;
use crate::loading::Loading;
use crate::theme::Palette;
use crate::widgets::Choice;

/// The floors offered: the rating both players had at least.
const RATINGS: [(u32, &str); 4] = [
    (2000, "Both 2000+"),
    (2200, "Both 2200+"),
    (2400, "Both 2400+"),
    (2500, "Both 2500+"),
];
pub const RATING: u32 = 2400;

/// "2026-10" as "October 2026".
fn month_label(month: &str) -> String {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let named = month.split_once('-').and_then(|(year, number)| {
        let name = NAMES.get(number.parse::<usize>().ok()?.checked_sub(1)?)?;
        Some(format!("{name} {year}"))
    });
    named.unwrap_or_else(|| month.to_string())
}

/// Opens the dialog for the floor `rating` (a Lichess database's, to add a month to it), or the
/// usual one.
pub fn open(rating: Option<u32>, window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| LichessDialog::new(rating.unwrap_or(RATING), window, cx));
    widgets::open_dialog(window, cx, move |dialog_box, _, _| {
        dialog_box
            .title("Download Lichess games")
            .w(px(460.))
            .child(dialog.clone())
    });
}

pub struct LichessDialog {
    rating: u32,
    rating_select: Entity<SelectState<Vec<Choice>>>,
    month_select: Entity<SelectState<Vec<Choice>>>,
    /// Lichess's months, once read.
    archive: Option<LichessArchive>,
    months: Loading,
    /// The month chosen, and how many bytes it downloads once Lichess says.
    month: Option<String>,
    size: Option<u64>,
    sizing: Option<Task<()>>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl LichessDialog {
    fn new(rating: u32, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ratings: Vec<Choice> = RATINGS
            .iter()
            .map(|(value, label)| Choice::new(value.to_string(), *label))
            .collect();
        let rating_select = cx.new(|cx| SelectState::new(ratings, None, window, cx));
        rating_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(rating.to_string()), window, cx)
        });
        let month_select = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let _subscriptions = vec![
            cx.subscribe_in(
                &rating_select,
                window,
                |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event
                        && let Ok(rating) = value.parse()
                    {
                        this.rating = rating;
                        this.show_months(window, cx);
                    }
                },
            ),
            cx.subscribe(
                &month_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.choose_month(Some(value.to_string()), cx);
                    }
                },
            ),
        ];
        let mut dialog = LichessDialog {
            rating,
            rating_select,
            month_select,
            archive: None,
            months: Loading::default(),
            month: None,
            size: None,
            sizing: None,
            error: None,
            _subscriptions,
        };
        dialog.load_months(window, cx);
        dialog
    }

    fn load_months(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let turn = self.months.start();
        let request = cx
            .global::<AppCore>()
            .request(calls::GameDbLichessMonths {});
        cx.spawn_in(window, async move |this, cx| {
            let answer = request.await;
            this.update_in(cx, |this, window, cx| {
                if !this.months.settle(turn, "game-db", &answer) {
                    return;
                }
                if let Ok(archive) = answer {
                    this.archive = Some(archive);
                    this.show_months(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The months the floor's database lacks, newest first.
    fn open_months(&self) -> Vec<String> {
        let Some(archive) = &self.archive else {
            return Vec::new();
        };
        let held = archive
            .held
            .iter()
            .find(|held| held.min_rating == self.rating)
            .map(|held| held.months.as_slice())
            .unwrap_or_default();
        archive
            .months
            .iter()
            .filter(|month| !held.contains(month))
            .cloned()
            .collect()
    }

    /// Lists the months the floor's database lacks, the newest of them chosen.
    fn show_months(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let months = self.open_months();
        let choices: Vec<Choice> = months
            .iter()
            .map(|month| Choice::new(month.clone(), month_label(month)))
            .collect();
        let chosen = self
            .month
            .clone()
            .filter(|month| months.contains(month))
            .or_else(|| months.first().cloned());
        self.month_select.update(cx, |state, cx| {
            state.set_items(choices, window, cx);
            if let Some(month) = &chosen {
                state.set_selected_value(&SharedString::from(month.clone()), window, cx);
            }
        });
        self.choose_month(chosen, cx);
    }

    /// Chooses `month`, and asks how large it is.
    fn choose_month(&mut self, month: Option<String>, cx: &mut Context<Self>) {
        self.error = None;
        if month != self.month || self.size.is_none() {
            self.size = None;
            self.sizing = month.clone().map(|month| {
                let request = cx.global::<AppCore>().request(calls::GameDbLichessSize {
                    month: LichessMonth(month),
                });
                cx.spawn(async move |this, cx| {
                    let size = request.await.warn_on_err().flatten();
                    this.update(cx, |this, cx| {
                        this.size = size;
                        cx.notify();
                    })
                    .ok();
                })
            });
        }
        self.month = month;
        cx.notify();
    }

    /// Starts the download, once the request is one the core takes, and closes the dialog: its
    /// progress shows on the page.
    fn download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = LichessGames {
            month: self.month.clone(),
            min_rating: self.rating,
        };
        let request = match kchess_contracts::check::lichess_games(&request) {
            Ok(request) => request,
            Err(refusal) => {
                self.error = Some(refusal.into());
                cx.notify();
                return;
            }
        };
        let db = cx.global::<GameDbHandle>().0.clone();
        db.update(cx, |db, cx| db.install_lichess(request, cx));
        window.close_dialog(cx);
    }
}

impl Render for LichessDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let busy = cx.global::<GameDbHandle>().0.read(cx).busy();
        let loaded = self.archive.is_some();
        let complete = loaded && self.month.is_none();
        let label = match self.size {
            Some(bytes) => SharedString::from(format!(
                "Download · {}",
                widgets::format_bytes(bytes as f64)
            )),
            None => SharedString::new_static("Download"),
        };
        let muted = |text: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(text)
        };
        let month: AnyElement = if self.months.busy() {
            widgets::loading("Loading months…", &palette).into_any_element()
        } else if complete {
            muted("Every month is in this database.").into_any_element()
        } else {
            div()
                .w(px(200.))
                .child(
                    Select::new(&self.month_select)
                        .small()
                        .accessibility_label("Month")
                        .disabled(!loaded),
                )
                .into_any_element()
        };
        let error = self
            .error
            .clone()
            .or_else(|| self.months.error().cloned())
            .map(|error| ("lichess-error", error));
        v_flex()
            .gap(px(12.))
            .child(
                h_flex().gap(px(8.)).child(
                    div().w(px(200.)).child(
                        Select::new(&self.rating_select)
                            .small()
                            .accessibility_label("Players rated"),
                    ),
                ),
            )
            .child(month)
            // Why a download of some 30 GB keeps so few games.
            .child(muted(
                "Only rated games of two players this strong are kept, without bullet or bots.",
            ))
            .child(widgets::dialog_footer(
                error,
                None,
                [
                    Button::new("lichess-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx))
                        .into_any_element(),
                    Button::new("lichess-download")
                        .primary()
                        .icon(kchess_ui::icon("download"))
                        .label(label)
                        // One download runs at a time.
                        .disabled(busy || self.month.is_none())
                        .on_click(cx.listener(|this, _, window, cx| this.download(window, cx)))
                        .into_any_element(),
                ],
                &palette,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::month_label;

    #[test]
    fn a_month_is_named_in_words() {
        assert_eq!(month_label("2026-10"), "October 2026");
        assert_eq!(month_label("2013-01"), "January 2013");
        assert_eq!(month_label("2026-13"), "2026-13");
    }
}
