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

/// `.moves-list`: the game's moves as a scoresheet, ruled numbered pairs on the panel itself, the
/// one on the board highlighted. `select` is called with the ply to show (1 for the first move).
pub fn moves_list(
    id: impl Into<ElementId>,
    sheet: MoveSheet<'_>,
    empty: SharedString,
    select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    figurine: crate::notation::Figurine,
    palette: &Palette,
) -> impl IntoElement {
    let MoveSheet {
        sans,
        shown: ply,
        first,
    } = sheet;
    let number_cell = |text: String| {
        div()
            .px(px(8.))
            .py(px(4.))
            .text_right()
            .text_color(palette.dimmed)
            .child(text)
    };
    let mut rows = Vec::new();
    let mut row: Vec<AnyElement> = Vec::new();
    for (index, san) in sans.iter().enumerate() {
        let at = move_at(index, first);
        if !at.black || index == 0 {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            row.push(number_cell(at.number.to_string()).into_any_element());
            if at.black {
                row.push(
                    div()
                        .flex_1()
                        .px(px(8.))
                        .py(px(4.))
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
                .test_support()
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .aria_label(san.clone())
                .aria_selected(active)
                .flex_1()
                .min_w_0()
                .px(px(8.))
                .py(px(4.))
                .text_center()
                .rounded(radii::XS)
                .cursor_pointer()
                .when(active, |cell| {
                    cell.bg(tint(palette.primary, 26.))
                        .font_weight(FontWeight::MEDIUM)
                })
                .pressable(palette)
                .on_click(move |_, window, cx| select(index + 1, window, cx))
                .child(crate::notation::shown(san, &figurine))
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
        .text_size(px(13.))
        .line_height(px(20.))
        .font_features(tabular())
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
        .when(!sans.is_empty(), |list| {
            list.border_t_1().border_color(palette.border)
        })
        .children(rows.into_iter().map(|cells| {
            let filled = cells.len();
            h_flex()
                .border_b_1()
                .border_color(palette.border)
                .children(cells.into_iter().enumerate().map(|(i, cell)| {
                    // The number column is 44px wide; the moves share the rest.
                    if i == 0 {
                        div().w(px(44.)).flex_none().child(cell).into_any_element()
                    } else {
                        cell
                    }
                }))
                .when(filled == 2, |row| row.child(div().flex_1().px(px(8.))))
        }))
}

/// `.move-controls`: first, previous, next and last move, and a board flip: its label, the key
/// that does the same on the page (named in the tooltip), and what a click does.
pub fn move_controls(
    ply: usize,
    count: usize,
    select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    flip: (SharedString, Option<String>, ClickHandler),
) -> impl IntoElement {
    let flip_tooltip: SharedString = match &flip.1 {
        Some(key) => format!("{} ({key})", flip.0).into(),
        None => flip.0.clone(),
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
                .tooltip(flip_tooltip)
                .accessibility_label(flip.0)
                .on_click(flip.2),
        )
}
/// The typed-move box's width on every page; it is as tall as a player's line.
const MOVE_ENTRY_WIDTH: Pixels = px(240.);

/// The typed-move box under a board: the input, dimmed
/// while no move can be made, and its Return hint.
pub fn move_entry(input: &Entity<InputState>, enabled: bool, palette: &Palette) -> Div {
    h_flex()
        .w(MOVE_ENTRY_WIDTH)
        .min_w_0()
        .h(PLAYER_LINE)
        .border_1()
        .border_color(palette.border)
        .rounded(radii::MD)
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

/// `.panel-actions`: the panel's buttons sharing one row equally; in a narrow panel two per row,
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

/// `.board-input-row`: the move box and the controls a page adds (voice) on one line, kept to the
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

/// `.player-middle` > `.move-entry`: the input row in a player's line.
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

    #[test]
    fn moves_are_numbered_from_the_game_s_first_move() {
        let setup = serde_json::json!({
            "variant": "standard",
            "fen": "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 3 30",
        });
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
        let usual = serde_json::json!({ "fen": crate::game::STANDARD_FEN });
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
