//! Pieces the board pages share, drawn as the Electron app draws them (`main.css`): the player
//! line above and below a board, the status banner and the move list panel.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;
use crate::theme::{Palette, tint};

/// What a button runs when it is clicked.
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// `.player-line` height.
pub const PLAYER_LINE: Pixels = px(36.);
/// The gap between the player lines and the board (`.board-stack`).
pub const STACK_GAP: Pixels = px(10.);

/// Which side a player plays, shown as a dot on their avatar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    White,
    Black,
}

/// `.player-avatar`: the player's icon, the side they play, and the outline while it is their turn.
pub fn avatar(glyph: Icon, side: Side, active: bool, palette: &Palette) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(30.))
        .rounded(px(8.))
        .bg(palette.accented)
        .text_color(palette.toned)
        .flex()
        .items_center()
        .justify_center()
        .child(glyph.size(px(16.)))
        .when(active, |avatar| {
            avatar.child(
                div()
                    .absolute()
                    .top(px(-2.))
                    .left(px(-2.))
                    .size(px(34.))
                    .rounded(px(10.))
                    .border_2()
                    .border_color(palette.primary),
            )
        })
        .child(
            div()
                .absolute()
                .right(px(-4.))
                .bottom(px(-4.))
                .size(px(13.))
                .rounded_full()
                .border_1()
                .border_color(palette.border_accented)
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .border_2()
                        .border_color(palette.bg)
                        .bg(match side {
                            Side::White => rgb(0xf4f4f4),
                            Side::Black => rgb(0x1c1c1c),
                        }),
                ),
        )
}

/// `.player-clock`: a side's time, highlighted while it runs.
pub fn clock(text: SharedString, active: bool, palette: &Palette) -> impl IntoElement {
    div()
        .min_w(px(72.))
        .px(px(10.))
        .py(px(4.))
        .rounded(px(8.))
        .text_right()
        .text_size(px(16.))
        .font_weight(FontWeight::BOLD)
        .bg(if active {
            palette.primary
        } else {
            palette.accented
        })
        .text_color(if active {
            palette.inverted
        } else {
            palette.text_muted
        })
        .child(text)
}

/// `.player-line`: who plays a side, what is beside them, and their clock.
pub fn player_line(
    identity: impl IntoElement,
    middle: Option<AnyElement>,
    aside: Option<AnyElement>,
) -> impl IntoElement {
    h_flex()
        .h(PLAYER_LINE)
        .flex_none()
        .px(px(2.))
        .gap(px(12.))
        .justify_between()
        .text_size(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(h_flex().min_w_0().gap(px(10.)).child(identity))
        .child(div().flex_1().min_w_0().children(middle))
        .children(aside)
}

/// The kind of a status banner (`.status-banner.win`, `.loss`, `.alert`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Win,
    Loss,
    Alert,
}

/// `.status-banner.steady`: the game's state, its icon and a detail line.
pub fn status_banner(
    tone: Tone,
    glyph: Icon,
    title: SharedString,
    detail: Option<SharedString>,
    extra: Option<AnyElement>,
    palette: &Palette,
) -> impl IntoElement {
    let (background, icon_color) = match tone {
        Tone::Plain => (palette.accented, palette.text),
        Tone::Win => (tint(palette.win, 18.), palette.win),
        Tone::Loss => (tint(palette.loss, 16.), palette.loss),
        Tone::Alert => (tint(palette.warning, 16.), palette.text),
    };
    h_flex()
        .items_center()
        .gap(px(10.))
        .min_h(px(58.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(10.))
        .text_size(px(13.))
        .bg(background)
        .child(glyph.size(px(16.)).flex_none().text_color(icon_color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .line_height(px(20.))
                        .font_weight(FontWeight(650.))
                        .text_color(palette.highlighted)
                        .child(title),
                )
                .children(detail.map(|detail| {
                    div()
                        .line_height(px(18.))
                        .text_size(px(12.))
                        .text_color(palette.toned)
                        .child(detail)
                }))
                .children(extra),
        )
}

/// One move of a list: its number, side and SAN.
struct Move {
    number: usize,
    black: bool,
}

fn move_at(index: usize, black_first: bool) -> Move {
    let ply = index + usize::from(black_first);
    Move {
        number: ply / 2 + 1,
        black: ply % 2 == 1,
    }
}

/// `.moves-list`: the game's moves in numbered pairs, the one on the board highlighted. `select`
/// is called with the ply to show (1 for the first move).
pub fn moves_list(
    id: impl Into<ElementId>,
    sans: &[String],
    ply: usize,
    black_first: bool,
    empty: SharedString,
    select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    let number_cell = |text: String| {
        div()
            .px(px(10.))
            .py(px(7.))
            .text_right()
            .text_color(palette.dimmed)
            .bg(tint(palette.elevated, 70.))
            .child(text)
    };
    let mut rows = Vec::new();
    let mut row: Vec<AnyElement> = Vec::new();
    for (index, san) in sans.iter().enumerate() {
        let at = move_at(index, black_first);
        if !at.black || index == 0 {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            row.push(number_cell(at.number.to_string()).into_any_element());
            if at.black {
                row.push(
                    div()
                        .flex_1()
                        .px(px(10.))
                        .py(px(7.))
                        .text_center()
                        .text_color(palette.text_muted)
                        .child("…")
                        .into_any_element(),
                );
            }
        }
        let active = ply == index + 1;
        let select = select.clone();
        row.push(
            div()
                .id(("move", index))
                .flex_1()
                .min_w_0()
                .px(px(10.))
                .py(px(7.))
                .text_center()
                .cursor_pointer()
                .when(active, |cell| {
                    cell.bg(tint(palette.primary, 26.))
                        .font_weight(FontWeight::BOLD)
                })
                .hover(|cell| cell.bg(palette.accented))
                .on_click(move |_, window, cx| select(index + 1, window, cx))
                .child(san.clone())
                .into_any_element(),
        );
    }
    if !row.is_empty() {
        rows.push(row);
    }
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .border_1()
        .border_color(palette.border)
        .rounded(px(10.))
        .bg(palette.bg)
        .text_size(px(13.))
        .when(sans.is_empty(), |list| {
            list.child(
                div()
                    .px(px(16.))
                    .py(px(28.))
                    .text_center()
                    .text_color(palette.text_muted)
                    .child(empty),
            )
        })
        .children(rows.into_iter().map(|cells| {
            let filled = cells.len();
            h_flex()
                .children(cells.into_iter().enumerate().map(|(i, cell)| {
                    // The number column is 44px wide; the moves share the rest.
                    if i == 0 {
                        div().w(px(44.)).flex_none().child(cell).into_any_element()
                    } else {
                        cell
                    }
                }))
                .when(filled == 2, |row| row.child(div().flex_1()))
        }))
}

/// `.move-controls`: first, previous, next and last move, and a board flip.
pub fn move_controls(
    ply: usize,
    count: usize,
    select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    flip: (SharedString, ClickHandler),
) -> impl IntoElement {
    let targets = [
        ("skip-back", "Go to start", 0, ply == 0),
        (
            "chevron-left",
            "Previous move",
            ply.saturating_sub(1),
            ply == 0,
        ),
        (
            "chevron-right",
            "Next move",
            (ply + 1).min(count),
            ply == count,
        ),
        ("skip-forward", "Go to last move", count, ply == count),
    ];
    let icons = [
        icon("skip-back"),
        icon("chevron-left"),
        icon("chevron-right"),
        icon("skip-forward"),
    ];
    h_flex()
        .justify_center()
        .gap(px(4.))
        .children(
            targets
                .into_iter()
                .zip(icons)
                .map(|((id, label, target, disabled), glyph)| {
                    let select = select.clone();
                    Button::new(id)
                        .ghost()
                        .small()
                        .icon(glyph)
                        .tooltip(label)
                        .accessibility_label(label)
                        .disabled(disabled)
                        .on_click(move |_, window, cx| select(target, window, cx))
                }),
        )
        .child(
            Button::new("flip")
                .ghost()
                .small()
                .icon(icon("arrow-up-down"))
                .tooltip(flip.0.clone())
                .accessibility_label(flip.0)
                .on_click(flip.1),
        )
}

/// `.card.side-panel`: the panel beside a board.
pub fn side_panel(palette: &Palette) -> Div {
    v_flex()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .p(px(16.))
        .gap(px(14.))
        .border_1()
        .border_color(palette.border)
        .rounded(px(14.))
        .bg(palette.elevated)
}

/// `.section-title`: a panel's or section's heading.
pub fn heading(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight(650.))
        .text_color(palette.highlighted)
        .child(text.into())
}

/// `.section-hint`: a short muted line under a heading.
pub fn hint(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(palette.text_muted)
        .child(text.into())
}

/// `.section-title` with a muted count beside it.
pub fn section_title(
    title: &'static str,
    count: Option<String>,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .items_baseline()
        .justify_between()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight(650.))
                .text_color(palette.highlighted)
                .child(title),
        )
        .children(count.map(|count| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(count)
        }))
}

/// `MaterialDiff.vue`: the pieces one side has won over the other, as silhouettes in the text
/// colour, and its lead in pawns, as Lichess shows them.
pub fn material(fen: &str, white: bool, set: &str, palette: &Palette) -> Option<AnyElement> {
    let diff = crate::game::rules("materialDiff", &[serde_json::json!(fen)]).ok()?;
    let roles: Vec<String> =
        serde_json::from_value(diff[if white { "white" } else { "black" }].clone())
            .unwrap_or_default();
    let lead = diff["score"].as_f64().unwrap_or_default() * if white { 1. } else { -1. };
    if roles.is_empty() && lead <= 0. {
        return None;
    }
    let mut groups: Vec<(String, usize)> = Vec::new();
    for role in roles {
        match groups.last_mut() {
            Some((last, count)) if *last == role => *count += 1,
            _ => groups.push((role, 1)),
        }
    }
    let letter = |role: &str| match role {
        "king" => 'K',
        "queen" => 'Q',
        "rook" => 'R',
        "bishop" => 'B',
        "knight" => 'N',
        _ => 'P',
    };
    let set = crate::board::looks::piece_set(set).to_string();
    Some(
        h_flex()
            .flex_none()
            .gap(px(4.))
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.toned)
            .children(groups.into_iter().map(|(role, count)| {
                let path: SharedString = format!("pieces/{set}/b{}.svg", letter(&role)).into();
                h_flex().children((0..count).map(move |n| {
                    svg()
                        .path(path.clone())
                        .size(px(16.))
                        .text_color(palette.toned)
                        .when(n > 0, |piece| piece.ml(px(-5.)))
                }))
            }))
            .when(lead > 0., |row| row.child(format!("+{lead}")))
            .into_any_element(),
    )
}

/// The blindfold and zen mode toggles every game page shows (`BoardViewToggles.vue`).
pub fn view_toggles(blindfold: bool, zen: bool) -> impl IntoElement {
    let toggle =
        |id: &'static str, glyph: gpui_kit::component::Icon, label: &'static str, on: bool| {
            Button::new(id)
                .xsmall()
                .when(on, |button| button.primary())
                .when(!on, |button| button.ghost())
                .icon(glyph)
                .tooltip(label)
                .accessibility_label(label)
        };
    h_flex()
        .gap(px(2.))
        .child(
            toggle(
                "blindfold",
                icon("eye-off"),
                "Blindfold: hide the pieces",
                blindfold,
            )
            .on_click(|_, _, cx| {
                crate::settings::change(cx, |s| s.frontend.blindfold = !s.frontend.blindfold)
            }),
        )
        .child(
            toggle("zen", icon("maximize"), "Zen mode (Z)", zen).on_click(|_, _, cx| {
                crate::settings::change(cx, |s| s.frontend.zen_mode = !s.frontend.zen_mode)
            }),
        )
}

/// A form field: its label above, and a problem below when there is one.
pub fn field(
    label: &'static str,
    control: impl IntoElement,
    problem: Option<SharedString>,
    palette: &Palette,
) -> Div {
    v_flex()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(label),
        )
        .child(control)
        .children(problem.map(|problem| {
            div()
                .text_size(px(13.))
                .text_color(palette.error)
                .child(problem)
        }))
}

/// A grid's columns for a width (`repeat(auto-fill, minmax(min, 1fr))`): as many as fit at
/// `min`, `gap` apart, sharing the width.
pub fn columns(width: Pixels, min: Pixels, gap: Pixels) -> (usize, Pixels) {
    let count = ((width + gap) / (min + gap)).floor().max(1.) as usize;
    let item = (width - gap * (count as f32 - 1.)) / count as f32;
    (count, item)
}

/// `.card`: a bordered, raised box with the page's standard padding.
pub fn card(palette: &Palette) -> Div {
    v_flex()
        .p(px(20.))
        .rounded(px(14.))
        .border_1()
        .border_color(palette.border)
        .bg(palette.elevated)
}

/// `UTabs variant="pill"`: a full-width row of choices, the chosen one filled with the primary
/// color. `choose` gets the index picked.
pub fn pill_tabs(
    id: &'static str,
    tabs: Vec<(Option<Icon>, SharedString)>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    pills(
        id,
        tabs.into_iter()
            .map(|(glyph, label)| Pill {
                icon: glyph,
                label,
                badge: None,
                disabled: false,
            })
            .collect(),
        selected,
        choose,
        false,
        palette,
    )
}

/// One choice of `pills`: its label, an icon or a count beside it, and whether it can be chosen.
pub struct Pill {
    pub icon: Option<Icon>,
    pub label: SharedString,
    pub badge: Option<SharedString>,
    pub disabled: bool,
}

/// `UTabs variant="pill"`, full width or (`compact`, `size="sm"`) as wide as its choices.
pub fn pills(
    id: &'static str,
    tabs: Vec<Pill>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    compact: bool,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .when(!compact, |row| row.w_full())
        .p(px(2.))
        .gap(px(4.))
        .rounded(px(8.))
        .border_1()
        .border_color(palette.border)
        .bg(palette.bg)
        .children(tabs.into_iter().enumerate().map(move |(index, tab)| {
            let chosen = index == selected;
            let choose = choose.clone();
            h_flex()
                .id((id, index))
                .when(!compact, |tab| tab.flex_1())
                .justify_center()
                .gap(px(if compact { 6. } else { 8. }))
                .h(px(if compact { 20. } else { 26. }))
                .px(px(if compact { 8. } else { 12. }))
                .rounded(px(6.))
                .text_size(px(if compact { 12. } else { 14. }))
                .font_weight(FontWeight::MEDIUM)
                .when(chosen, |tab| {
                    tab.bg(palette.primary).text_color(palette.inverted)
                })
                .when(!chosen && tab.disabled, |tab| {
                    tab.text_color(palette.text_muted).opacity(0.75)
                })
                .when(!chosen && !tab.disabled, |tab| {
                    tab.cursor_pointer()
                        .text_color(palette.text_muted)
                        .hover(|tab| tab.text_color(palette.highlighted))
                })
                .when(!tab.disabled, |row| {
                    row.on_click(move |_, window, cx| choose(index, window, cx))
                })
                .children(tab.icon.map(|glyph| glyph.size(px(18.))))
                .child(tab.label)
                .children(tab.badge.map(|badge| {
                    div()
                        .px(px(5.))
                        .rounded(px(4.))
                        .text_size(px(10.))
                        .line_height(px(16.))
                        .when(chosen, |badge| {
                            badge.bg(palette.bg).text_color(palette.highlighted)
                        })
                        .when(!chosen, |badge| {
                            badge.bg(palette.elevated).text_color(palette.toned)
                        })
                        .child(badge)
                }))
        }))
}

/// The typed-move box under a board (`.board-move-form` in `ChessBoard.vue`): the input, dimmed
/// while no move can be made, and its Return hint.
pub fn move_entry(input: &Entity<InputState>, enabled: bool, palette: &Palette) -> Div {
    h_flex()
        .border_1()
        .border_color(palette.border)
        .rounded(px(8.))
        .bg(palette.bg)
        .when(!enabled, |entry| entry.opacity(0.55))
        .child(
            div().flex_1().min_w_0().text_size(px(13.)).child(
                Input::new(input)
                    .appearance(false)
                    .small()
                    .disabled(!enabled)
                    .aria_label("Enter a chess move in SAN or UCI"),
            ),
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
        )
}

/// `UEmpty`: an icon, a title and an optional line under it; the caller adds the actions.
pub fn empty_state(
    glyph: Icon,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    palette: &Palette,
) -> Div {
    v_flex()
        .items_center()
        .gap(px(10.))
        .child(glyph.size(px(24.)).text_color(palette.text_muted))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title.into()),
        )
        .children(description.map(|description| {
            div()
                .text_size(px(14.))
                .text_center()
                .text_color(palette.text_muted)
                .child(description)
        }))
}

/// `.side-dot`: a piece colour, the same light or dark disc in either theme.
pub fn side_dot(color: shakmaty::Color) -> Div {
    div()
        .size(px(9.))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(rgb(0x6b6b6b))
        .bg(rgb(if color == shakmaty::Color::White {
            0xf4f4f2
        } else {
            0x1f1f1f
        }))
}

/// A select's entry, known by its value: two entries may share a label (a study and an opening).
#[derive(Clone)]
pub struct Choice {
    pub value: SharedString,
    pub label: SharedString,
}

impl gpui_kit::component::searchable_list::SearchableListItem for Choice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.value
    }
}

impl Choice {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Choice {
        Choice {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// What `StudyCard.vue` shows: a study or an opening as a card in a grid.
pub struct StudyCard {
    pub id: String,
    pub name: String,
    /// The position as a small board; a book stands in when there is none.
    pub board: Option<AnyView>,
    /// The first detail line (where it is kept, or an opening's code).
    pub location: String,
    /// The second line (its players and event, or an opening's moves).
    pub description: Option<String>,
    /// "Edited 2 days ago".
    pub edited: Option<String>,
    /// Detail rows kept for every card, so cards in a grid line up (3 with dates, 2 without).
    pub rows: usize,
    pub menu: Option<AnyElement>,
    pub action: AnyElement,
}

/// `StudyCard.vue`; clicking its picture or its name opens it.
pub fn study_card(
    card: StudyCard,
    width: Pixels,
    open: impl Fn(&mut Window, &mut App) + 'static,
    palette: &Palette,
) -> AnyElement {
    let open = std::rc::Rc::new(open);
    let open_title = open.clone();
    let row = |text: Option<String>| {
        div()
            .h(px(15.))
            .text_size(px(12.))
            .text_color(palette.text_muted)
            .truncate()
            .children(text)
    };
    let mut details = vec![
        row(Some(card.location)),
        row(card.description),
        row(card.edited),
    ];
    details.truncate(card.rows);
    v_flex()
        .id(SharedString::from(format!("card-{}", card.id)))
        .w(width)
        .overflow_hidden()
        .border_1()
        .border_color(palette.border)
        .rounded(px(12.))
        .bg(palette.elevated)
        .child(
            div()
                .id(SharedString::from(format!("card-picture-{}", card.id)))
                .w_full()
                .aspect_ratio(1.)
                .flex()
                .items_center()
                .justify_center()
                .bg(palette.bg)
                .cursor_pointer()
                .on_click(move |_, window, cx| open(window, cx))
                .child(match card.board {
                    Some(board) => div().size_full().child(board).into_any_element(),
                    None => icon("book-open")
                        .size(px(48.))
                        .text_color(palette.text_muted)
                        .into_any_element(),
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .p(px(12.))
                .gap(px(8.))
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(8.))
                        .child(
                            div()
                                .id(SharedString::from(format!("card-name-{}", card.id)))
                                .flex_1()
                                .min_w_0()
                                .min_h(px(44.))
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .line_height(relative(1.4))
                                .line_clamp(2)
                                .cursor_pointer()
                                .on_click(move |_, window, cx| open_title(window, cx))
                                .child(card.name),
                        )
                        .children(card.menu),
                )
                .child(v_flex().gap(px(8.)).children(details))
                .child(div().mt_auto().pt(px(4.)).child(card.action)),
        )
        .into_any_element()
}

/// A notification with one action (Undo, Open) that closes it when used.
pub fn action_notification(
    note: gpui_kit::component::notification::Notification,
    id: &'static str,
    label: &'static str,
    run: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui_kit::component::notification::Notification {
    let run = std::rc::Rc::new(run);
    note.action(move |_, _, cx| {
        let (run, note) = (run.clone(), cx.entity().downgrade());
        Button::new(id).label(label).on_click(move |_, window, cx| {
            run(window, cx);
            let _ = note.update(cx, |note, cx| note.dismiss(window, cx));
        })
    })
}

/// `formatCount`: "100,000".
pub fn format_count(count: f64) -> String {
    let digits = (count.max(0.).round() as u64).to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `formatBytes`: "312 MB", "1.5 GB".
pub fn format_bytes(bytes: f64) -> String {
    if !bytes.is_finite() || bytes <= 0. {
        return "0 B".into();
    }
    let units = ["B", "KB", "MB", "GB"];
    let exponent = ((bytes.ln() / 1024f64.ln()).floor() as usize).min(units.len() - 1);
    let value = bytes / 1024f64.powi(exponent as i32);
    if exponent == 0 || value >= 100. {
        format!("{} {}", value.round(), units[exponent])
    } else {
        format!("{value:.1} {}", units[exponent])
    }
}

/// How a badge is drawn (`UBadge` color and variant).
#[derive(Clone, Copy)]
pub enum Look {
    Success,
    Error,
    Warning,
    Primary,
    Info,
    NeutralSoft,
    NeutralSubtle,
}

/// `UBadge`: a short label in a soft color; callers add an icon, a width or a dot.
pub fn badge(label: impl Into<SharedString>, look: Look, palette: &Palette) -> Div {
    let (bg, fg, ring) = match look {
        Look::Success => (tint(palette.success, 10.), palette.success, None),
        Look::Error => (tint(palette.error, 10.), palette.error, None),
        Look::Warning => (tint(palette.warning, 10.), palette.warning, None),
        Look::Primary => (tint(palette.primary, 10.), palette.primary, None),
        Look::Info => (tint(palette.info, 10.), palette.info, None),
        Look::NeutralSoft => (palette.elevated, palette.toned, None),
        Look::NeutralSubtle => (
            palette.elevated,
            palette.toned,
            Some(palette.border_accented),
        ),
    };
    h_flex()
        .flex_none()
        .gap(px(6.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(6.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .when_some(ring, |badge, ring| badge.border_1().border_color(ring))
        .child(label.into())
}

/// `ResultBar.vue`: a label, the score (wins plus half the draws), the games, and a bar of won,
/// drawn and lost.
pub fn result_row(
    label: impl Into<SharedString>,
    hint: Option<String>,
    (win, draw, loss): (f64, f64, f64),
    palette: &Palette,
) -> impl IntoElement {
    let label = label.into();
    let total = win + draw + loss;
    let score = if total > 0. {
        (100. * (win + draw / 2.) / total).round()
    } else {
        0.
    };
    let games = format!(
        "{} game{}",
        format_count(total),
        if total == 1. { "" } else { "s" }
    );
    let title = SharedString::from(format!(
        "{label}: {games} · {win} won, {draw} drawn, {loss} lost · score {score}%"
    ));
    let parts = [
        (win, palette.win),
        (draw, palette.border_accented),
        (loss, palette.loss),
    ];
    v_flex()
        .gap(px(4.))
        .py(px(3.))
        .text_size(px(13.))
        .child(
            h_flex()
                .gap(px(12.))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().child(label.clone()))
                        .children(hint.map(|hint| {
                            div()
                                .flex_none()
                                .text_color(palette.text_muted)
                                .child(format!(" · {hint}"))
                        })),
                )
                .child(
                    h_flex()
                        .gap(px(3.))
                        .items_baseline()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if total > 0. {
                            format!("{score}%")
                        } else {
                            "–".into()
                        })
                        .when(total > 0., |value| {
                            value.child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(palette.text_muted)
                                    .child("score"),
                            )
                        }),
                )
                .child(
                    div()
                        .w(px(88.))
                        .text_right()
                        .text_color(palette.text_muted)
                        .child(games),
                ),
        )
        .child(
            h_flex()
                .id(SharedString::from(format!("result-row-{label}")))
                .h(px(8.))
                .rounded(px(4.))
                .overflow_hidden()
                .bg(palette.elevated)
                .when(total > 0., |bar| {
                    bar.tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                })
                .children(parts.into_iter().filter(|(part, _)| *part > 0.).map(
                    move |(part, color)| {
                        div().h_full().w(relative((part / total) as f32)).bg(color)
                    },
                )),
        )
}
