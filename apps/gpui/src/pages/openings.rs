//! Openings (`pages/openings.vue`): the browser (`OpeningsBrowser.vue`) with a search, the
//! openings opened lately, popular ones and every family by first move; and one opening
//! (`OpeningsDetail.vue`) on a board to explore, with its moves, its Wikibooks description and
//! the ways to train it. The names and the tree are the shared rules' (`openingTree`).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shakmaty::{Color, Position as _};

use crate::assets::icon;
use crate::board::{BoardEvent, BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::{STANDARD_FEN, rules, view};
use crate::pages::Page;
use crate::play_layout::{
    Measured, PAGE_BOTTOM, PAGE_TOP, PANEL_MAX, PANEL_MIN, PlayLayout, side_padding,
};
use crate::theme::Palette;
use crate::widgets::{self, Pill};

const GROUP: &str = "kchess:openings-group";
const RECENT: &str = "kchess:recent-openings";
const RECENT_LIMIT: usize = 6;
/// Search results shown at once.
const LIMIT: usize = 60;
const SEARCH_DELAY: Duration = Duration::from_millis(120);
/// The popular cards' columns (`.popular-grid`).
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

/// One entry of the openings tree (`OpeningNode`).
#[derive(Clone, Deserialize)]
struct OpeningNode {
    name: String,
    label: String,
    eco: Option<String>,
    moves: Option<Vec<String>>,
    children: Vec<OpeningNode>,
}

/// An opening chosen to drill or opened lately (`OpeningChoice`).
#[derive(Clone, Serialize, Deserialize)]
pub struct OpeningChoice {
    pub name: String,
    pub moves: Vec<String>,
}

/// An opening as a card shows it: its name and code, its position and its moves.
#[derive(Clone)]
struct Entry {
    name: String,
    eco: String,
    moves: Vec<String>,
    fen: String,
    san: String,
}

/// The moves as far as they are legal, each with its SAN and the position after it.
fn line_of(moves: &[String]) -> Vec<(String, String, String)> {
    let Some(replay) = kchess_domain::replay::replay_setup("standard", STANDARD_FEN, moves) else {
        return Vec::new();
    };
    let positions = kchess_domain::replay::replay_positions(STANDARD_FEN, moves);
    replay
        .played
        .into_iter()
        .zip(positions.into_iter().skip(1))
        .map(|(played, position)| (played.uci, played.san, position.fen))
        .collect()
}

/// `openingAt`: the name and code of the line.
fn opening_at(moves: &[String]) -> Option<(String, String)> {
    let named = rules(
        "openingAt",
        &[
            json!({ "variant": "standard", "fen": STANDARD_FEN }),
            json!(moves),
            json!(moves.len()),
        ],
    )
    .ok()?;
    Some((
        named["name"].as_str()?.to_string(),
        named["eco"].as_str().unwrap_or_default().to_string(),
    ))
}

fn entry(moves: &[String], name: Option<&str>) -> Option<Entry> {
    let steps = line_of(moves);
    let (_, _, fen) = steps.last()?.clone();
    let line: Vec<String> = steps.iter().map(|(uci, _, _)| uci.clone()).collect();
    let (opening, eco) = opening_at(&line)?;
    let san = steps
        .iter()
        .enumerate()
        .map(|(index, (_, san, _))| {
            if index % 2 == 0 {
                format!("{}.{san}", index / 2 + 1)
            } else {
                san.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(Entry {
        name: name.map_or(opening, str::to_string),
        eco,
        moves: line,
        fen,
        san,
    })
}

/// Search ignores case, accents and the kind of apostrophe.
fn fold(text: &str) -> String {
    text.chars()
        .map(|letter| match letter {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            '’' | '`' => '\'',
            letter => letter,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn first_move(node: &OpeningNode) -> Option<&str> {
    node.moves
        .as_ref()
        .and_then(|moves| moves.first().map(String::as_str))
        .or_else(|| node.children.iter().find_map(first_move))
}

fn group_of(node: &OpeningNode) -> &'static str {
    let first = first_move(node);
    GROUPS
        .iter()
        .find(|(_, _, moves)| first.is_some_and(|first| moves.contains(&first)))
        .map_or("other", |(key, _, _)| key)
}

/// The named openings, shortest line first, with their folded search text.
fn named(tree: &[OpeningNode]) -> Vec<(String, String, Vec<String>, String)> {
    fn walk(nodes: &[OpeningNode], out: &mut Vec<(String, String, Vec<String>, String)>) {
        for node in nodes {
            if let Some(moves) = &node.moves {
                let eco = node.eco.clone().unwrap_or_default();
                out.push((
                    node.name.clone(),
                    eco.clone(),
                    moves.clone(),
                    fold(&format!("{} {eco}", node.name)),
                ));
            }
            walk(&node.children, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out.sort_by(|a, b| a.2.len().cmp(&b.2.len()).then_with(|| a.3.cmp(&b.3)));
    out
}

pub struct OpeningsPage {
    tree: Vec<OpeningNode>,
    named: Vec<(String, String, Vec<String>, String)>,
    popular: Vec<Entry>,
    search: Entity<InputState>,
    query: String,
    search_task: Option<Task<()>>,
    /// The browser starts afresh once a line was opened from it, as Electron's page is remade.
    clear_search: bool,
    expanded: HashSet<String>,
    previews: HashMap<String, Entity<ChessBoard>>,
    area: Measured,
    /// The opening shown, when one is: its moves from the starting position.
    line: Option<Vec<String>>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    orientation: Color,
    move_input: Entity<InputState>,
    description: Option<Value>,
    full_description: bool,
    description_task: Option<Task<()>>,
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
        let tree: Vec<OpeningNode> = rules("openingTree", &[])
            .ok()
            .and_then(|tree| serde_json::from_value(tree).ok())
            .unwrap_or_default();
        let named = named(&tree);
        let popular = POPULAR
            .iter()
            .filter_map(|line| {
                let moves: Vec<String> = line.split(' ').map(str::to_string).collect();
                entry(&moves, None)
            })
            .collect();
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search openings or ECO codes"));
        let move_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a move: Nf3 or g1f3"));
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
                Color::White,
                Some(Color::White),
                None,
            ))
        });
        let subscriptions = vec![
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
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    let mut moves = this.line.clone().unwrap_or_default();
                    moves.push(uci.clone());
                    this.show_line(Some(moves), cx);
                }
            }),
            cx.subscribe_in(
                &move_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.typed_move(window, cx);
                    }
                },
            ),
        ];
        OpeningsPage {
            tree,
            named,
            popular,
            search,
            query: String::new(),
            search_task: None,
            clear_search: false,
            expanded: HashSet::new(),
            previews: HashMap::new(),
            area: Measured::default(),
            line: None,
            board,
            layout: PlayLayout::default(),
            orientation: Color::White,
            move_input,
            description: None,
            full_description: false,
            description_task: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
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
    pub fn show_line(&mut self, moves: Option<Vec<String>>, cx: &mut Context<Self>) {
        self.description = None;
        self.full_description = false;
        self.description_task = None;
        let Some(moves) = moves else {
            self.line = None;
            cx.notify();
            return;
        };
        if self.line.is_none() {
            self.clear_search = true;
        }
        // The moves as far as they are legal.
        let steps = line_of(&moves);
        let line: Vec<String> = steps.iter().map(|(uci, _, _)| uci.clone()).collect();
        if let Some((name, _)) = opening_at(&line) {
            Self::remember(&name, &line, cx);
        }
        if !line.is_empty() {
            let description = cx
                .global::<AppCore>()
                .call("openingDescription", vec![json!(line)]);
            self.description_task = Some(cx.spawn(async move |this, cx| match description.await {
                Ok(page) => {
                    let _ = this.update(cx, |this, cx| {
                        this.description = (!page.is_null()).then_some(page);
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
        self.line = Some(line);
        self.redraw(cx);
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let line = self.line.clone().unwrap_or_default();
        let fen = line_of(&line)
            .last()
            .map_or_else(|| STANDARD_FEN.to_string(), |(_, _, fen)| fen.clone());
        let position = kchess_domain::rules::setup_start("standard", &fen).unwrap_or_default();
        let last = line
            .last()
            .and_then(|uci| Some((uci.get(0..2)?.parse().ok()?, uci.get(2..4)?.parse().ok()?)));
        let state = BoardState::of(&position, self.orientation, Some(position.turn()), last);
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.move_input.read(cx).value().trim().to_string();
        let line = self.line.clone().unwrap_or_default();
        let fen = line_of(&line)
            .last()
            .map_or_else(|| STANDARD_FEN.to_string(), |(_, _, fen)| fen.clone());
        let setup = json!({ "variant": "standard", "fen": fen });
        let typed = kchess_domain::rules::setup_start("standard", &fen)
            .and_then(|position| view::typed_uci(&setup, &position, &text));
        if let Some(uci) = typed {
            self.move_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            let mut moves = line;
            moves.push(uci);
            self.show_line(Some(moves), cx);
        }
    }

    /// `useTrainOpening`: Practice → Repertoire on an opening.
    fn train(choice: OpeningChoice, window: &mut Window, cx: &mut App) {
        crate::prefs::set(cx, "kchess:repertoire-opening", &choice);
        crate::prefs::set(cx, "kchess:repertoire-study", "opening");
        crate::prefs::set(cx, "kchess:practice-tab", "openings");
        crate::shell::open(Page::Practice, window, cx);
    }

    fn render_card(
        &mut self,
        item: &Entry,
        width: Pixels,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let board = self.preview(&item.fen, cx);
        let (open, train) = (item.moves.clone(), item.clone());
        let page = cx.entity().downgrade();
        v_flex()
            .id(SharedString::from(format!("card-{}", item.name)))
            .w(width)
            .overflow_hidden()
            .border_1()
            .border_color(palette.border)
            .rounded(px(12.))
            .bg(palette.elevated)
            .child(
                div()
                    .id(SharedString::from(format!("card-board-{}", item.name)))
                    .cursor_pointer()
                    .on_click({
                        let page = page.clone();
                        let open = open.clone();
                        move |_, _, cx| {
                            let _ =
                                page.update(cx, |page, cx| page.show_line(Some(open.clone()), cx));
                        }
                    })
                    .child(board),
            )
            .child(
                v_flex()
                    .flex_1()
                    .p(px(12.))
                    .gap(px(8.))
                    .child(
                        div()
                            .id(SharedString::from(format!("card-name-{}", item.name)))
                            .min_h(px(44.))
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(relative(1.4))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                let _ = page
                                    .update(cx, |page, cx| page.show_line(Some(open.clone()), cx));
                            })
                            .child(item.name.clone()),
                    )
                    .child(
                        v_flex()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(item.eco.clone())
                            .child(div().truncate().child(item.san.clone())),
                    )
                    .child(
                        Button::new(SharedString::from(format!("train-{}", item.name)))
                            .outline()
                            .small()
                            .h(px(28.))
                            .w_full()
                            .icon(icon("graduation-cap"))
                            .label("Train")
                            .accessibility_label(format!("Train {}", item.name))
                            .on_click(move |_, window, cx| {
                                Self::train(
                                    OpeningChoice {
                                        name: train.name.clone(),
                                        moves: train.moves.clone(),
                                    },
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    /// `OpeningsTreeNode.vue`: one entry, with its variations below it once opened.
    fn render_node(
        &self,
        node: &OpeningNode,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let open = self.expanded.contains(&node.name);
        let name = node.name.clone();
        let toggle = {
            let name = name.clone();
            cx.listener(
                move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                    if !this.expanded.remove(&name) {
                        this.expanded.insert(name.clone());
                    }
                    cx.notify();
                },
            )
        };
        let row = h_flex()
            .gap(px(6.))
            .min_h(px(28.))
            .when(!node.children.is_empty(), |row| {
                row.child(
                    Button::new(SharedString::from(format!("toggle-{name}")))
                        .ghost()
                        .xsmall()
                        .icon(if open {
                            icon("chevron-down")
                        } else {
                            icon("chevron-right")
                        })
                        .accessibility_label(format!(
                            "{} {} variations",
                            if open { "Hide" } else { "Show" },
                            node.name
                        ))
                        .on_click(toggle),
                )
            })
            .when(node.children.is_empty(), |row| {
                row.child(div().w(px(24.)).flex_none())
            })
            .child({
                let link = div()
                    .id(SharedString::from(format!("open-{name}")))
                    .min_w_0()
                    .truncate()
                    .cursor_pointer()
                    .hover(|link| link.underline())
                    .child(node.label.clone());
                match &node.moves {
                    Some(moves) => {
                        let moves = moves.clone();
                        link.on_click(cx.listener(move |this, _, _, cx| {
                            this.show_line(Some(moves.clone()), cx)
                        }))
                    }
                    None => {
                        let name = name.clone();
                        link.on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&name) {
                                this.expanded.insert(name.clone());
                            }
                            cx.notify();
                        }))
                    }
                }
            })
            .children(node.eco.clone().map(|eco| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(eco)
            }));
        v_flex()
            .child(row)
            .when(open, |entry| {
                entry.child(
                    v_flex()
                        .ml(px(14.))
                        .pl(px(10.))
                        .border_l_1()
                        .border_color(palette.border)
                        .children(
                            node.children
                                .iter()
                                .map(|child| self.render_node(child, cx, palette))
                                .collect::<Vec<_>>(),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_browser(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let section = |title: &'static str| {
            div()
                .mb(px(8.))
                .text_size(px(15.))
                .font_weight(FontWeight(650.))
                .text_color(palette.highlighted)
                .child(title)
        };
        let words: Vec<String> = fold(&self.query)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let mut body = v_flex()
            .gap(px(24.))
            .w_full()
            .max_w(px(1100.))
            .child(Input::new(&self.search).prefix(icon("search")));
        if !words.is_empty() {
            let matches: Vec<_> = self
                .named
                .iter()
                .filter(|(_, _, _, key)| words.iter().all(|word| key.contains(word.as_str())))
                .collect();
            let total = matches.len();
            let rows: Vec<AnyElement> = matches
                .into_iter()
                .take(LIMIT)
                .map(|(name, eco, moves, _)| {
                    let moves = moves.clone();
                    h_flex()
                        .id(SharedString::from(format!("result-{name}")))
                        .justify_between()
                        .gap(px(12.))
                        .py(px(4.))
                        .child(
                            div()
                                .min_w_0()
                                .cursor_pointer()
                                .hover(|link| link.underline())
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(eco.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_line(Some(moves.clone()), cx)
                        }))
                        .into_any_element()
                })
                .collect();
            body = body.child(
                widgets::card(palette)
                    .when(total == 0, |card| {
                        card.child(
                            div()
                                .text_size(px(14.))
                                .text_color(palette.text_muted)
                                .child("No openings found."),
                        )
                    })
                    .children(rows)
                    .when(total > LIMIT, |card| {
                        card.child(
                            div()
                                .mt(px(8.))
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(format!("{LIMIT} of {} openings", total)),
                        )
                    }),
            );
            return body.into_any_element();
        }
        let width = self.area.size().width;
        let width = if width > px(0.) {
            width.min(px(1100.))
        } else {
            window.viewport_size().width - px(400.)
        };
        let recent: Vec<Entry> = Self::recent(cx)
            .iter()
            .filter_map(|choice| entry(&choice.moves, Some(&choice.name)))
            .collect();
        if !recent.is_empty() {
            let (count, item) = widgets::columns(width, px(220.), px(8.));
            let mut rows = Vec::new();
            let items: Vec<AnyElement> = recent
                .iter()
                .map(|recent| {
                    let board = self.preview(&recent.fen, cx);
                    let moves = recent.moves.clone();
                    h_flex()
                        .id(SharedString::from(format!("recent-{}", recent.name)))
                        .w(item)
                        .gap(px(10.))
                        .pl(px(6.))
                        .pr(px(10.))
                        .py(px(6.))
                        .border_1()
                        .border_color(palette.border)
                        .rounded(px(10.))
                        .bg(palette.elevated)
                        .cursor_pointer()
                        .hover(|item| item.bg(palette.accented))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_line(Some(moves.clone()), cx)
                        }))
                        .child(div().w(px(44.)).flex_none().child(board))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(recent.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(recent.eco.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            let mut items = items.into_iter().peekable();
            while items.peek().is_some() {
                rows.push(h_flex().gap(px(8.)).children(items.by_ref().take(count)));
            }
            body = body.child(
                v_flex()
                    .child(section("Recent"))
                    .child(v_flex().gap(px(8.)).children(rows)),
            );
        }
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let popular = self.popular.clone();
        let cards: Vec<AnyElement> = popular
            .iter()
            .map(|item| self.render_card(item, card_width, cx, palette))
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
        body = body.child(
            v_flex()
                .child(section("Popular"))
                .child(v_flex().gap(CARD_GAP).children(rows)),
        );
        let group = crate::prefs::get::<String>(cx, GROUP, "e4".into());
        let families: Vec<AnyElement> = self
            .tree
            .iter()
            .filter(|node| group_of(node) == group)
            .map(|node| self.render_node(node, cx, palette))
            .collect();
        body = body.child(
            v_flex()
                .child(section("All openings"))
                .child(
                    div().mb(px(12.)).child(widgets::pills(
                        "openings-group",
                        GROUPS
                            .iter()
                            .map(|(_, label, _)| Pill {
                                icon: None,
                                label: (*label).into(),
                                badge: None,
                                disabled: false,
                            })
                            .collect(),
                        GROUPS
                            .iter()
                            .position(|(key, _, _)| *key == group)
                            .unwrap_or(0),
                        |index, window, cx| {
                            crate::prefs::set(cx, GROUP, GROUPS[index].0);
                            window.refresh();
                        },
                        true,
                        palette,
                    )),
                )
                .child(widgets::card(palette).children(families)),
        );
        body.into_any_element()
    }

    fn render_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let line = self.line.clone().unwrap_or_default();
        let steps = line_of(&line);
        let opening = opening_at(&line);
        let title = opening
            .as_ref()
            .map_or_else(|| "Starting position".to_string(), |(name, _)| name.clone());
        let board_size = self.layout.board_size(window);
        let entry = h_flex()
            .h(px(36.))
            .border_1()
            .border_color(palette.border)
            .rounded(px(8.))
            .bg(palette.bg)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.move_input).appearance(false).small()),
            )
            .child(
                div()
                    .w(px(32.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_color(palette.border)
                    .text_color(palette.text_muted)
                    .child("↵"),
            );
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(self.board.clone())
            .child(entry);
        let moves = h_flex()
            .flex_wrap()
            .gap_x(px(6.))
            .gap_y(px(2.))
            .text_size(px(14.))
            .children(steps.iter().enumerate().map(|(index, (_, san, _))| {
                let upto = line[..=index].to_vec();
                let current = index + 1 == steps.len();
                h_flex()
                    .gap(px(4.))
                    .when(index % 2 == 0, |row| {
                        row.child(
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("{}.", index / 2 + 1)),
                        )
                    })
                    .child(
                        div()
                            .id(("opening-move", index))
                            .px(px(3.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(current, |chip| chip.font_weight(FontWeight::SEMIBOLD))
                            .hover(|chip| chip.bg(palette.accented))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.show_line(Some(upto.clone()), cx)
                            }))
                            .child(san.clone()),
                    )
            }));
        let train_choice = OpeningChoice {
            name: title.clone(),
            moves: line.clone(),
        };
        let analyse_line = line.clone();
        let puzzles = opening.clone();
        let description = self.description.clone().map(|page| {
            let blocks: Vec<Value> = page["blocks"].as_array().cloned().unwrap_or_default();
            let shown: Vec<Value> = if self.full_description {
                blocks.clone()
            } else {
                blocks
                    .iter()
                    .find(|block| block["heading"] != json!(true))
                    .cloned()
                    .into_iter()
                    .collect()
            };
            let more = blocks.len() > shown.len() || self.full_description;
            let url = page["url"].as_str().unwrap_or_default().to_string();
            v_flex()
                .gap(px(6.))
                .children(shown.into_iter().map(|block| {
                    div()
                        .text_size(px(14.))
                        .when(block["heading"] == json!(true), |text| {
                            text.font_weight(FontWeight::SEMIBOLD)
                        })
                        .child(block["text"].as_str().unwrap_or_default().to_string())
                }))
                .when(more, |section| {
                    section.child(
                        h_flex().child(
                            Button::new("description-more")
                                .link()
                                .xsmall()
                                .label(if self.full_description {
                                    "Less"
                                } else {
                                    "More"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.full_description = !this.full_description;
                                    cx.notify();
                                })),
                        ),
                    )
                })
                .child(
                    h_flex()
                        .gap(px(4.))
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child("Excerpt from")
                        .child(
                            div()
                                .id("wikibooks")
                                .underline()
                                .cursor_pointer()
                                .on_click(move |_, _, cx| cx.open_url(&url))
                                .child("Wikibooks"),
                        )
                        .child("·")
                        .child(
                            div()
                                .id("cc-by-sa")
                                .underline()
                                .cursor_pointer()
                                .on_click(|_, _, cx| {
                                    cx.open_url("https://creativecommons.org/licenses/by-sa/4.0/")
                                })
                                .child("CC BY-SA 4.0"),
                        ),
                )
        });
        let panel = widgets::side_panel(palette)
            .h(board_size + px(36.) + widgets::STACK_GAP)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("all-openings")
                            .ghost()
                            .small()
                            .icon(icon("arrow-left"))
                            .label("All openings")
                            .on_click(cx.listener(|this, _, _, cx| this.show_line(None, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("openings-flip")
                            .ghost()
                            .xsmall()
                            .icon(icon("arrow-up-down"))
                            .tooltip("Flip board")
                            .accessibility_label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.orientation = !this.orientation;
                                this.redraw(cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight(650.))
                            .text_color(palette.highlighted)
                            .child(title),
                    )
                    .children(opening.as_ref().map(|(_, eco)| {
                        div()
                            .text_size(px(14.))
                            .text_color(palette.text_muted)
                            .child(eco.clone())
                    })),
            )
            .when(!steps.is_empty(), |panel| panel.child(moves))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("train-line")
                            .primary()
                            .icon(icon("graduation-cap"))
                            .label("Train this line")
                            .disabled(steps.is_empty())
                            .on_click(move |_, window, cx| {
                                Self::train(train_choice.clone(), window, cx)
                            }),
                    )
                    .child(
                        Button::new("analyse-line")
                            .outline()
                            .icon(icon("microscope"))
                            .label("Analyse")
                            .on_click(move |_, window, cx| {
                                crate::shell::open_analysis_line(analyse_line.clone(), window, cx)
                            }),
                    )
                    .children(puzzles.map(|(name, _)| {
                        Button::new("train-puzzles")
                            .outline()
                            .icon(icon("puzzle"))
                            .label("Train with puzzles")
                            .on_click(move |_, window, cx| {
                                crate::shell::open_opening_puzzles(&name, window, cx)
                            })
                    })),
            )
            .children(description);
        self.layout
            .render("opening-detail", board_column, panel, window, cx)
            .into_any_element()
    }
}

impl Render for OpeningsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.line.is_some() {
            return div()
                .size_full()
                .track_focus(&self.focus)
                .child(self.render_detail(window, cx, &palette))
                .into_any_element();
        }
        if std::mem::take(&mut self.clear_search) {
            self.query.clear();
            self.search_task = None;
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
        }
        let browser = self.render_browser(window, cx, &palette);
        div()
            .id("openings")
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

#[cfg(test)]
mod tests {
    use super::{OpeningNode, entry, fold, group_of};

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
