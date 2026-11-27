//! The move list panel and the move entry beneath it.

use super::*;

/// A move of a list: its number and whether Black plays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstMove {
    pub number: usize,
    pub black: bool,
}

impl Default for FirstMove {
    fn default() -> Self {
        Self {
            number: 1,
            black: false,
        }
    }
}

/// The move at `index` of a list that begins with `first`.
fn move_at(index: usize, first: FirstMove) -> FirstMove {
    let ply = index + usize::from(first.black);
    FirstMove {
        number: first.number + ply / 2,
        black: ply % 2 == 1,
    }
}

/// The moves a scoresheet lists (`moves_list`): their SAN, the ply on the board (0 before the
/// first move) and the game's `first` move, which numbers them.
pub struct MoveSheet<'a> {
    pub sans: &'a [String],
    pub shown: usize,
    pub first: FirstMove,
}

/// The number column of every move sheet, the analysis board's main line too.
pub const SHEET_NUMBER_WIDTH: Pixels = px(36.);

/// A sheet's text: 13 px on 20 px lines, in figures of one width so the moves line up.
pub fn sheet_text<E: Styled>(sheet: E) -> E {
    sheet
        .text_size(px(13.))
        .line_height(px(20.))
        .font_features(tabular())
}

/// A row's move number, right-aligned in the number column.
pub fn sheet_number(
    id: impl Into<ElementId>,
    number: usize,
    palette: &Palette,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .w(SHEET_NUMBER_WIDTH)
        .flex_none()
        .pr(px(8.))
        .text_right()
        .text_color(palette.dimmed)
        .child(number.to_string())
}

/// "…" in a row's move cell that has no move: Black's first in a game Black starts, or
/// White's line broken by a comment or a variation before Black's reply.
pub fn sheet_gap(palette: &Palette) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .px(px(8.))
        .text_color(palette.text_muted)
        .child("…")
}

/// A move as a button, named for the move and tinted while it is on the board: a sheet's cell
/// (`sheet_move`), or a move in an analysis variation. The page adds what a click does and what
/// it shows.
pub fn move_button(
    id: impl Into<ElementId>,
    label: SharedString,
    active: bool,
    palette: &Palette,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Button)
        .keyboard_focus(palette.primary)
        .aria_label(label)
        .aria_selected(active)
        .rounded(radii::SMALL)
        .cursor_pointer()
        .when(active, |cell| {
            cell.bg(tint(palette.primary, 26.))
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |cell| cell.pressable(palette))
}

/// A move's cell in a sheet's row, sharing the row with the other side's move.
pub fn sheet_move(
    id: impl Into<ElementId>,
    label: SharedString,
    active: bool,
    palette: &Palette,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    move_button(id, label, active, palette)
        .flex_1()
        .min_w_0()
        .px(px(8.))
        .py(px(2.))
}

/// The `index`th numbered row of a sheet, compact and, every other one, lightly striped rather
/// than ruled: its number and up to two moves, an empty cell keeping White's move as wide alone
/// as beside Black's.
pub fn sheet_row(cells: Vec<AnyElement>, index: usize, palette: &Palette) -> Div {
    let filled = cells.len();
    h_flex()
        .h(ROW)
        .gap(px(2.))
        .rounded(radii::SMALL)
        .when(index % 2 == 1, |row| row.bg(palette.accented.opacity(0.45)))
        .children(cells)
        .when(filled == 2, |row| row.child(div().flex_1().px(px(8.))))
}

/// Drawn over the move on the board: once `due` says it was just shown, scrolls `scroll` the
/// least that brings the move into view with an 8 px margin (`block: 'nearest'`).
pub fn reveal(scroll: ScrollHandle, due: impl FnOnce() -> bool + 'static) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            if !due() {
                return;
            }
            let view = scroll.bounds();
            let mut offset = scroll.offset();
            let margin = px(8.);
            if bounds.bottom() + margin > view.bottom() {
                offset.y -= bounds.bottom() + margin - view.bottom();
            } else if bounds.top() - margin < view.top() {
                offset.y += view.top() - (bounds.top() - margin);
            } else {
                return;
            }
            scroll.set_offset(offset);
            window.refresh();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// The game's moves as a scoresheet, compact numbered pairs (`sheet_row`) on the panel itself, the
/// one on the board highlighted and kept in view as it changes, the newest as it is played.
/// `select` is called with the ply to show (1 for the first move).
pub fn moves_list(
    id: impl Into<ElementId>,
    sheet: MoveSheet<'_>,
    empty: SharedString,
    select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    figurine: crate::notation::Figurine,
    palette: &Palette,
) -> MovesList {
    MovesList {
        id: id.into(),
        sans: sheet.sans.to_vec(),
        shown: sheet.shown,
        first: sheet.first,
        empty,
        select: std::rc::Rc::new(select),
        figurine,
        palette: *palette,
    }
}

/// What a list keeps from frame to frame: its scroll, and the ply it last brought into view.
#[derive(Default)]
struct Follow {
    scroll: ScrollHandle,
    revealed: std::rc::Rc<std::cell::Cell<Option<usize>>>,
}

/// What choosing a move does: called with the ply to show.
type SelectPly = std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A move list (`moves_list`), owning what it shows.
#[derive(IntoElement)]
pub struct MovesList {
    id: ElementId,
    sans: Vec<String>,
    shown: usize,
    first: FirstMove,
    empty: SharedString,
    select: SelectPly,
    figurine: crate::notation::Figurine,
    palette: Palette,
}

impl RenderOnce for MovesList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let MovesList {
            id,
            sans,
            shown: ply,
            first,
            empty,
            select,
            figurine,
            palette,
        } = self;
        let palette = &palette;
        let follow = window.use_keyed_state(
            ElementId::NamedChild(std::sync::Arc::new(id.clone()), "follow".into()),
            cx,
            |_, _| Follow::default(),
        );
        let (scroll, revealed) = {
            let follow = follow.read(cx);
            (follow.scroll.clone(), follow.revealed.clone())
        };
        let mut rows = Vec::new();
        let mut row: Vec<AnyElement> = Vec::new();
        for (index, san) in sans.iter().enumerate() {
            let at = move_at(index, first);
            if !at.black || index == 0 {
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
                row.push(
                    sheet_number(("move-number", index), at.number, palette).into_any_element(),
                );
                if at.black {
                    row.push(sheet_gap(palette).into_any_element());
                }
            }
            let active = ply == index + 1;
            // At the start, the first move is brought into view: the top of the list.
            let revealing = ply.max(1) == index + 1;
            let select = select.clone();
            row.push(
                sheet_move(("move", index), san.clone().into(), active, palette)
                    .on_click(move |_, window, cx| select(index + 1, window, cx))
                    .child(crate::notation::shown(san, &figurine))
                    .when(revealing, |cell| {
                        let (scroll, revealed) = (scroll.clone(), revealed.clone());
                        cell.relative().child(reveal(scroll, move || {
                            revealed.replace(Some(ply)) != Some(ply)
                        }))
                    })
                    .into_any_element(),
            );
        }
        if !row.is_empty() {
            rows.push(row);
        }
        sheet_text(
            div()
                .id(id)
                .test_support()
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&scroll),
        )
        .when(sans.is_empty(), |list| {
            list.child(
                div()
                    .id("moves-empty")
                    .test_support()
                    .aria_label(empty.clone())
                    .text_color(palette.text_muted)
                    .child(empty),
            )
        })
        .children(
            rows.into_iter()
                .enumerate()
                .map(|(index, cells)| sheet_row(cells, index, palette)),
        )
    }
}

/// A pair's row in a sheet.
const ROW: Pixels = px(26.);

/// The button after the move controls: Flip board, whose key (F) its tooltip names, or another
/// turn of the board the page names (Switch sides).
pub enum TurnButton {
    Flip(ClickHandler),
    Named(SharedString, ClickHandler),
}

/// First, previous, next and last move, and the board's turn.
pub fn move_controls(
    ply: usize,
    count: usize,
    select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    turn: TurnButton,
    cx: &App,
) -> impl IntoElement {
    let (label, tooltip, click): (SharedString, SharedString, _) = match turn {
        TurnButton::Flip(click) => ("Flip board".into(), crate::flip_label(cx).into(), click),
        TurnButton::Named(label, click) => (label.clone(), label, click),
    };
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
                .tooltip(tooltip)
                .accessibility_label(label)
                .on_click(click),
        )
}
/// The typed-move box's width on every page; it is as tall as a player's line.
const MOVE_ENTRY_WIDTH: Pixels = px(240.);

/// The typed-move box under a board: the input, dimmed
/// while no move can be made, and its Return hint.
pub fn move_entry(input: &Entity<InputState>, enabled: bool, palette: &Palette) -> Div {
    text_entry(
        input,
        enabled,
        "Enter a chess move in SAN or UCI",
        true,
        palette,
    )
}

/// A box in the move box's place and look, named `label`, for what a page has typed there (a
/// square in Coordinates); `submit` shows the Return hint where Return enters it.
pub fn text_entry(
    input: &Entity<InputState>,
    enabled: bool,
    label: &'static str,
    submit: bool,
    palette: &Palette,
) -> Div {
    h_flex()
        .w(MOVE_ENTRY_WIDTH)
        .min_w_0()
        .h(PLAYER_LINE)
        .border_1()
        .border_color(palette.border)
        .rounded(radii::BOX)
        .bg(palette.bg)
        .when(!enabled, |entry| entry.opacity(0.55))
        .child(
            div().flex_1().min_w_0().text_size(px(13.)).child(
                Input::new(input)
                    .appearance(false)
                    .small()
                    .disabled(!enabled)
                    .aria_label(label),
            ),
        )
        .when(submit, |entry| {
            entry.child(
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
        })
}

/// `@container side-panel (max-width: 400px)`: a panel this narrow puts two actions per row.
const NARROW_PANEL: Pixels = px(400.);

/// A side panel measured `width` wide on the outside (zero before its first layout) is narrow:
/// the container query reads its content box, inside the 16px padding and 1px border.
pub fn is_narrow(width: Pixels) -> bool {
    width > px(0.) && width - px(34.) <= NARROW_PANEL
}

/// One of a panel's actions: filling its share of the row, or (`fit`) only as wide as it is.
pub struct Action(AnyElement, bool);

impl From<AnyElement> for Action {
    fn from(element: AnyElement) -> Action {
        Action(element, true)
    }
}

/// An action kept to its own width (an icon button such as Flip).
pub fn fit(element: impl IntoElement) -> Action {
    Action(element.into_any_element(), false)
}

/// The panel's buttons sharing one row equally; in a narrow panel two per row,
/// an odd one out taking the whole row.
pub fn panel_actions(narrow: bool, actions: impl IntoIterator<Item = Action>) -> Div {
    h_flex()
        .w_full()
        .gap(px(8.))
        .when(narrow, |row| row.flex_wrap())
        .children(actions.into_iter().map(|Action(element, fill)| {
            h_flex()
                .min_w_0()
                .justify_center()
                .when(fill, |cell| cell.flex_1())
                .when(!fill, |cell| cell.flex_none())
                .when(narrow && fill, |cell| cell.min_w(relative(0.45)))
                .child(element)
        }))
}

/// The move box and the controls a page adds (voice) on one line, kept to the
/// right, the controls at most 60% of the line.
pub fn input_row(entry: Div, controls: impl IntoElement) -> Div {
    entry_line(entry, Some(controls.into_any_element()))
}

/// The move box alone on its line, on a page with no controls beside it.
pub fn lone_entry(entry: Div) -> Div {
    entry_line(entry, None)
}

fn entry_line(entry: Div, controls: Option<AnyElement>) -> Div {
    h_flex().w_full().min_w_0().justify_end().child(
        h_flex()
            .flex_1()
            .min_w_0()
            .gap(px(8.))
            .justify_end()
            .child(entry)
            .children(controls.map(|controls| {
                div()
                    .flex_none()
                    .max_w(relative(0.6))
                    .min_w_0()
                    .child(controls)
            })),
    )
}

/// The input row in a player's line.
pub fn move_row(entry: Div, controls: impl IntoElement) -> AnyElement {
    h_flex()
        .flex_1()
        .min_w_0()
        .child(input_row(entry, controls))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{FirstMove, move_at};

    /// The list numbered every game from 1: one set up at Black's 30th move showed "1. … Nf6"
    /// where the scoresheet reads "30. … Nf6".
    #[test]
    fn moves_are_numbered_from_the_game_s_first_move() {
        let setup = kchess_domain::variant::Setup::standard_at(
            "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 3 30",
        );
        let first = crate::game::view::first_move(&setup);
        assert_eq!(
            first,
            FirstMove {
                number: 30,
                black: true
            }
        );
        let numbered: Vec<_> = (0..3).map(|index| move_at(index, first)).collect();
        assert_eq!(
            numbered,
            [
                FirstMove {
                    number: 30,
                    black: true
                },
                FirstMove {
                    number: 31,
                    black: false
                },
                FirstMove {
                    number: 31,
                    black: true
                },
            ]
        );
        let usual = kchess_domain::variant::Setup::standard();
        assert_eq!(crate::game::view::first_move(&usual), FirstMove::default());
        assert_eq!(
            move_at(2, FirstMove::default()),
            FirstMove {
                number: 2,
                black: false
            }
        );
    }
}
