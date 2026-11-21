//! Openings: the browser with a search, the
//! openings opened lately, popular ones and every family by first move; and one opening
//! on a board to explore, with its moves, its Wikibooks description and
//! the ways to train it. The names and the tree are the shared rules'.

use crate::pages::PageView;
use kchess_contracts::calls;
use kchess_contracts::check::Moves;
use kchess_contracts::types::{DescriptionBlock, OpeningDescription, PopularityMonth};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_domain::training::openings::{OpeningNode, opening_tree};
use kchess_domain::variant::Variant;
use shakmaty::{Color, Position as _};

use crate::assets::icon;
use crate::board::{BoardEvent, BoardState, ChessBoard};
use crate::chart::{ChartPoint, LineChart};
use crate::core::AppCore;
use crate::explorer::{ExplorerEvent, PositionExplorer};
use crate::game::STANDARD_FEN;
use crate::game::surface::TakesKeys as _;
use crate::pages::Page;
use crate::play_layout::{
    Measured, PAGE_BOTTOM, PAGE_TOP, PANEL_MAX, PANEL_MIN, PlayLayout, side_padding,
};
use crate::quick_pick;
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, KeyboardFocus as _, Pill};

const GROUP: &str = "kchess:openings-group";
const RECENT: &str = "kchess:recent-openings";
const RECENT_LIMIT: usize = 6;
/// Search results shown at once.
const LIMIT: usize = 60;
const SEARCH_DELAY: Duration = Duration::from_millis(120);
/// The popular cards' columns.
const CARD_MIN: Pixels = px(150.);
const CARD_GAP: Pixels = px(16.);

/// Widely played openings, by the line that names each.
const POPULAR: [&str; 12] = [
    "e2e4 c7c5",
    "e2e4 e7e6",
    "e2e4 c7c6",
    "e2e4 e7e5 g1f3 b8c6 f1b5",
    "e2e4 e7e5 g1f3 b8c6 f1c4",
    "e2e4 d7d5",
    "d2d4 d7d5 c2c4",
    "d2d4 d7d5 c2c4 c7c6",
    "d2d4 g8f6 c2c4 g7g6 b1c3",
    "d2d4 g8f6 c2c4 e7e6 b1c3 f8b4",
    "d2d4 d7d5 g1f3 g8f6 c1f4",
    "c2c4",
];

/// Every family under the first move of its shortest named line.
const GROUPS: [(&str, &str, &[&str]); 4] = [
    ("e4", "1.e4", &["e2e4"]),
    ("d4", "1.d4", &["d2d4"]),
    ("flank", "1.c4 / 1.Nf3", &["c2c4", "g1f3"]),
    ("other", "Other", &[]),
];

// An opening chosen to drill or opened lately, as Practice keeps it too (`kchess_board::game`).
pub use crate::game::OpeningChoice;

mod catalog;
mod draw;
use catalog::*;

/// An open line's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Train,
    Analyse,
    Puzzles,
    Flip,
    AllOpenings,
}

pub struct OpeningsPage {
    catalog: Arc<Catalog>,
    search: Entity<InputState>,
    query: String,
    search_task: Option<Task<()>>,
    /// A line was just opened from the list: its page takes the keys from the search field.
    focus_line: bool,
    expanded: HashSet<String>,
    previews: crate::board::Previews,
    area: Measured,
    /// The opening shown, when one is: its moves from the starting position.
    line: Option<Vec<String>>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    orientation: Color,
    move_box: crate::game::surface::MoveBox,
    description: Option<OpeningDescription>,
    full_description: bool,
    description_task: Option<Task<()>>,
    /// Month by month, how often the line's last move was chosen; shown once there is a year of it.
    popularity: Entity<LineChart>,
    popularity_months: usize,
    popularity_san: String,
    popularity_task: Option<Task<()>>,
    /// Popular continuations as a grid of boards (`PositionExplorer grid`).
    explorer: Entity<PositionExplorer>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for OpeningsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl OpeningsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> OpeningsPage {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search openings or ECO codes"));
        let (move_box, typed) = crate::game::surface::MoveBox::new(Self::typed_move, window, cx);
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start(
                    kchess_domain::variant::Variant::Standard,
                    STANDARD_FEN,
                )
                .unwrap_or_default(),
                Color::White,
                Some(Color::White),
                None,
            ))
        });
        let explorer =
            cx.new(|cx| PositionExplorer::new(crate::explorer::Layout::Grid, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &explorer,
                window,
                |this, _, event: &ExplorerEvent, window, cx| {
                    let ExplorerEvent::Play(uci) = event;
                    let mut moves = this.line.clone().unwrap_or_default();
                    moves.push(uci.clone());
                    this.show_line(Some(moves), window, cx);
                },
            ),
            cx.subscribe(&search, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = state.read(cx).value().to_string();
                    this.search_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SEARCH_DELAY).await;
                        let _ = this.update(cx, |this, cx| {
                            this.query = text;
                            cx.notify();
                        });
                    }));
                }
            }),
            cx.subscribe_in(&board, window, |this, _, event: &BoardEvent, window, cx| {
                if let BoardEvent::Move(uci) = event {
                    let mut moves = this.line.clone().unwrap_or_default();
                    moves.push(uci.clone());
                    this.show_line(Some(moves), window, cx);
                }
            }),
            typed,
        ];
        OpeningsPage {
            catalog: catalog(),
            search,
            query: String::new(),
            search_task: None,
            focus_line: false,
            expanded: HashSet::new(),
            previews: Default::default(),
            area: Measured::default(),
            line: None,
            board,
            layout: PlayLayout::default(),
            orientation: Color::White,
            move_box,
            description: None,
            full_description: false,
            description_task: None,
            popularity: cx.new(|_| LineChart::new(px(110.), percent)),
            popularity_months: 0,
            popularity_san: String::new(),
            popularity_task: None,
            explorer,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The preview boards kept, by position.
    #[cfg(test)]
    pub(crate) fn kept_previews(&self) -> std::collections::BTreeMap<String, gpui_kit::EntityId> {
        self.previews
            .kept()
            .map(|(fen, board)| (fen.clone(), board.entity_id()))
            .collect()
    }

    /// A small board picturing a position, made once per position.
    fn preview(&mut self, fen: &str, cx: &mut Context<Self>) -> Entity<ChessBoard> {
        let state = || BoardState::preview(fen, Default::default(), Color::White, None);
        self.previews.board(fen.to_string(), state, cx)
    }

    fn recent(cx: &App) -> Vec<OpeningChoice> {
        crate::prefs::get::<Vec<OpeningChoice>>(cx, RECENT, Vec::new())
    }

    /// The landing page lists the named openings shown here most recently.
    fn remember(name: &str, moves: &[String], cx: &mut App) {
        let mut list = Self::recent(cx);
        list.retain(|choice| choice.name != name);
        list.insert(
            0,
            OpeningChoice {
                name: name.to_string(),
                moves: moves.to_vec(),
            },
        );
        list.truncate(RECENT_LIMIT);
        crate::prefs::set(cx, RECENT, list);
    }

    /// Show an opening (its line), or the browser (`None`).
    pub fn show_line(
        &mut self,
        moves: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.description = None;
        self.full_description = false;
        self.description_task = None;
        self.popularity_months = 0;
        self.popularity_task = None;
        let Some(moves) = moves else {
            self.line = None;
            cx.notify();
            return;
        };
        // Leaving the browser for a line: back in it, every opening shows again.
        if self.line.is_none() {
            self.query.clear();
            self.search_task = None;
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.focus_line = true;
        }
        // The moves as far as they are legal.
        let steps = line_of(&moves);
        let line: Vec<String> = steps.iter().map(|(uci, _, _)| uci.clone()).collect();
        if let Some((name, _)) = opening_at(&line) {
            Self::remember(&name, &line, cx);
        }
        if !line.is_empty() {
            let description = cx.global::<AppCore>().request(calls::OpeningDescription {
                moves: Moves(line.clone()),
            });
            self.description_task = Some(cx.spawn(async move |this, cx| match description.await {
                Ok(page) => {
                    let _ = this.update(cx, |this, cx| {
                        this.description = page;
                        cx.notify();
                    });
                }
                Err(cause) => crate::log::write(
                    Level::Warn,
                    "openings",
                    &format!("Loading the description failed: {}", cause.message),
                ),
            }));
        }
        if !line.is_empty() {
            let months = cx.global::<AppCore>().request(calls::OpeningPopularity {
                moves: Moves(line.clone()),
            });
            let san = steps
                .last()
                .map(|(_, san, _)| san.clone())
                .unwrap_or_default();
            self.popularity_task = Some(cx.spawn(async move |this, cx| match months.await {
                Ok(months) => {
                    let _ = this.update(cx, |this, cx| this.show_popularity(&months, &san, cx));
                }
                Err(cause) => crate::log::write(
                    Level::Warn,
                    "openings",
                    &format!("Loading the popularity failed: {}", cause.message),
                ),
            }));
        }
        let fen = steps
            .last()
            .map_or_else(|| STANDARD_FEN.to_string(), |(_, _, fen)| fen.clone());
        let shown = line.clone();
        self.explorer.update(cx, |explorer, cx| {
            explorer.show(Variant::Standard, &fen, Some(shown), false, cx)
        });
        self.line = Some(line);
        self.redraw(cx);
    }

    /// Each month's share of games, once there is a year of them.
    fn show_popularity(&mut self, months: &[PopularityMonth], san: &str, cx: &mut Context<Self>) {
        let points: Vec<ChartPoint> = months
            .iter()
            .filter_map(|month| {
                let key = &month.month;
                let date = jiff::civil::Date::strptime("%Y-%m-%d", format!("{key}-01")).ok()?;
                Some(ChartPoint {
                    value: (month.share * 1000.).round() / 10.,
                    short: date.strftime("%b %y").to_string().into(),
                    long: date.strftime("%b %-d, %Y").to_string().into(),
                })
            })
            .collect();
        self.popularity_months = if points.len() >= 12 { points.len() } else { 0 };
        if self.popularity_months == 0 {
            cx.notify();
            return;
        }
        let top = points.iter().map(|point| point.value).fold(1., f64::max);
        let (first, last) = (&points[0], &points[points.len() - 1]);
        let first_month = &months[0].month;
        let last_month = &months[points.len() - 1].month;
        let summary = format!(
            "{san} was chosen in {} of games in {first_month} and {} in {last_month}",
            percent(first.value),
            percent(last.value)
        );
        self.popularity_san = san.to_string();
        self.popularity.update(cx, |chart, cx| {
            chart.set(points, (0., (top * 1.15).min(100.)), summary, cx)
        });
        cx.notify();
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let line = self.line.clone().unwrap_or_default();
        let fen = line_of(&line)
            .last()
            .map_or_else(|| STANDARD_FEN.to_string(), |(_, _, fen)| fen.clone());
        let position =
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, &fen)
                .unwrap_or_default();
        let last = line
            .last()
            .and_then(|uci| Some((uci.get(0..2)?.parse().ok()?, uci.get(2..4)?.parse().ok()?)));
        let state = BoardState::of(&position, self.orientation, Some(position.turn()), last);
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.line.clone().unwrap_or_default();
        let fen = line_of(&line)
            .last()
            .map_or_else(|| STANDARD_FEN.to_string(), |(_, _, fen)| fen.clone());
        let variant = kchess_domain::variant::Variant::Standard;
        let position = kchess_domain::rules::setup_start(variant, &fen);
        if let Some(uci) = self.move_box.take(position.as_ref(), window, cx) {
            let mut moves = line;
            moves.push(uci);
            self.show_line(Some(moves), window, cx);
        }
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        let line = self.line.clone().unwrap_or_default();
        let opening = opening_at(&line);
        match action {
            Action::Train => {
                let name =
                    opening.map_or_else(|| "Starting position".to_string(), |(name, _)| name);
                Self::train(OpeningChoice { name, moves: line }, window, cx);
            }
            Action::Analyse => {
                kchess_board::analysis::open(kchess_board::analysis::Open::Line(line), window, cx)
            }
            Action::Puzzles => {
                if let Some((name, _)) = opening {
                    crate::pages::puzzles::open_opening(&name, window, cx);
                }
            }
            Action::Flip => self.flip(cx),
            Action::AllOpenings => self.show_line(None, window, cx),
        }
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        self.orientation = !self.orientation;
        self.redraw(cx);
    }

    /// Practice → Repertoire on an opening.
    fn train(choice: OpeningChoice, window: &mut Window, cx: &mut App) {
        crate::pages::practice::repertoire::train_opening(&choice, window, cx);
    }
}

impl Render for OpeningsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.line.is_some() {
            if std::mem::take(&mut self.focus_line) {
                window.focus(&self.focus, cx);
            }
            return div()
                .size_full()
                .track_focus(&self.focus)
                // Escape goes back to all openings.
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if !crate::board::page_has_keys(&this.focus, window, cx)
                        || event.keystroke.modifiers.modified()
                    {
                        return;
                    }
                    if event.keystroke.key == "escape" {
                        this.show_line(None, window, cx);
                        cx.stop_propagation();
                    }
                }))
                .takes_keys(&self.focus)
                .child(self.render_detail(window, cx, &palette))
                .into_any_element();
        }
        let browser = self.render_browser(window, cx, &palette);
        div()
            .id("openings")
            .test_support()
            .size_full()
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .child(
                h_flex()
                    .justify_center()
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .max_w(px(1100.))
                            .child(self.area.probe(cx))
                            .child(browser),
                    ),
            )
            .into_any_element()
    }
}

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::ActionInfo;
        match self {
            Action::Train => ActionInfo::new(
                "openings:train",
                "Train this line",
                "graduation-cap",
                "repertoire drill",
            ),
            Action::Analyse => {
                ActionInfo::new("openings:analyse", "Analyse", "microscope", "analysis")
            }
            Action::Puzzles => ActionInfo::new(
                "openings:puzzles",
                "Train with puzzles",
                "puzzle",
                "tactics",
            ),
            Action::Flip => ActionInfo::flip("openings:flip"),
            Action::AllOpenings => {
                ActionInfo::new("openings:all", "All openings", "arrow-left", "back browse")
            }
        }
    }
}

impl PageView for OpeningsPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: Page::Openings,
        label: "Openings",
        short: "Openings",
        icon: "book-open-text",
        group: Some(crate::pages::NavGroup::Train),
        keywords: "eco gambit defense variation explorer repertoire",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        OpeningsPage::new(window, cx)
    }

    /// The sidebar's Openings is the browser, not the line shown last.
    fn shown(this: &Entity<Self>, _again: bool, window: &mut Window, cx: &mut App) {
        this.update(cx, |openings, cx| openings.show_line(None, window, cx));
    }

    fn search_fields(&self) -> Vec<Entity<InputState>> {
        vec![self.search.clone()]
    }

    fn actions(&self, _: &App) -> Vec<Action> {
        let Some(line) = &self.line else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if !line_of(line).is_empty() {
            actions.push(Action::Train);
        }
        actions.push(Action::Analyse);
        if opening_at(line).is_some() {
            actions.push(Action::Puzzles);
        }
        actions.extend([Action::Flip, Action::AllOpenings]);
        actions
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{OpeningNode, catalog, entry, fold, fold_char, group_of, opening_tree};

    /// A result lights what its search found, accents and case aside.
    #[test]
    fn results_light_what_the_search_found() {
        assert_eq!(fold_char('É'), 'e');
        assert_eq!(fold_char('’'), '\'');
        assert_eq!(fold_char('x'), 'x');
        let words: Vec<&str> = vec!["reti"];
        assert_eq!(
            crate::quick_pick::found("Réti Opening", &words, fold_char),
            vec![(0, 3)]
        );
    }

    /// Each Openings page made the catalog again as it opened, from a JSON copy of the rules'
    /// tree.
    #[test]
    fn every_page_shares_one_catalog() {
        let first = catalog();
        assert!(Arc::ptr_eq(&first, &catalog()));
        assert!(
            std::ptr::eq(first.tree, opening_tree().as_slice()),
            "the rules' own tree"
        );
        assert!(!first.tree.is_empty());
        assert!(!first.named.is_empty());
        assert!(!first.popular.is_empty());
    }

    #[test]
    fn search_ignores_accents_and_apostrophes() {
        assert_eq!(fold("Grünfeld Defense"), "grunfeld defense");
        assert_eq!(fold("King’s Gambit"), "king's gambit");
    }

    #[test]
    fn a_card_names_its_line() {
        let moves: Vec<String> = ["e2e4", "c7c5"].map(String::from).to_vec();
        let card = entry(&moves, None).unwrap();
        assert_eq!(card.name, "Sicilian Defense");
        assert_eq!(card.eco, "B20");
        assert_eq!(card.san, "1.e4 c5");
    }

    #[test]
    fn families_go_under_their_first_move() {
        let node = OpeningNode {
            name: "English Opening".into(),
            label: "English Opening".into(),
            eco: Some("A10".into()),
            moves: Some(vec!["c2c4".into()]),
            children: Vec::new(),
        };
        assert_eq!(group_of(&node), "flank");
    }
}
