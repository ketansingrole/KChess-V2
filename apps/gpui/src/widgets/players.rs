//! The player line above and below a board: names, avatars, clocks and material.

use super::*;

/// A player line's height.
pub const PLAYER_LINE: Pixels = px(36.);
/// The gap between the player lines and the board.
pub const STACK_GAP: Pixels = px(8.);

/// Figures of one width, so clocks and move numbers hold still.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

/// A large ticking time at `size` (the chess clock's faces, Puzzle Rush, a timed practice run):
/// tabular figures, so the numbers hold still as they tick instead of shifting with each digit's
/// width.
pub fn timer(text: impl Into<SharedString>, size: Pixels) -> Div {
    div()
        .text_size(size)
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.))
        .font_features(tabular())
        .child(text.into())
}

/// Which side a player plays, shown as a dot on their avatar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    White,
    Black,
}

/// A player's first two letters in a circle `size` across, where Lichess has no picture.
pub fn initials(name: &str, size: f32, palette: &Palette) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(palette.accented)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px((size * 0.4).round()))
        .font_weight(FontWeight::MEDIUM)
        .child(name.chars().take(2).collect::<String>().to_uppercase())
}

/// The player's icon, the side they play, and the outline while it is their turn.
pub fn avatar(glyph: Icon, side: Side, active: bool, palette: &Palette) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(30.))
        .rounded(radii::MD)
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
                    .rounded(radii::LG)
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
                .border_color(palette.dimmed)
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
/// A side's time, highlighted while it runs, and red in its last ten seconds.
pub fn clock(text: SharedString, active: bool, palette: &Palette) -> Div {
    let low = low_time(&text);
    div()
        .min_w(px(72.))
        .px(px(8.))
        .py(px(4.))
        .rounded(radii::MD)
        // Centred in its 72px: a short time ("5:00") right-aligned left a gap on its left. Its
        // tabular digits keep it still as it counts down.
        .text_center()
        .text_size(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .font_features(tabular())
        .bg(match (active, low) {
            (true, true) => palette.error,
            (true, false) => palette.primary,
            (false, _) => palette.accented,
        })
        .text_color(match (active, low) {
            (true, _) => palette.inverted,
            (false, true) => palette.error,
            (false, false) => palette.text_muted,
        })
        .child(text)
}

/// Who plays a side, what is beside them, and their clock.
pub fn player_line(
    identity: impl IntoElement,
    middle: Option<AnyElement>,
    aside: Option<AnyElement>,
) -> impl IntoElement {
    h_flex()
        .h(PLAYER_LINE)
        .flex_none()
        .gap(px(12.))
        .justify_between()
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .child(h_flex().min_w_0().gap(px(8.)).child(identity))
        .child(div().flex_1().min_w_0().children(middle))
        .children(aside)
}
/// The pieces one side has won over the other, as silhouettes in the text
/// colour, and its lead in pawns, as Lichess shows them.
pub fn material(fen: &str, white: bool, set: &str, palette: &Palette) -> Option<AnyElement> {
    let diff = kchess_domain::voice::material_diff(fen);
    let roles = if white { diff.white } else { diff.black };
    let lead = diff.score as f64 * if white { 1. } else { -1. };
    if roles.is_empty() && lead <= 0. {
        return None;
    }
    let mut groups: Vec<(String, usize)> = Vec::new();
    for role in roles {
        match groups.last_mut() {
            Some((last, count)) if *last == role => *count += 1,
            _ => groups.push((role.to_string(), 1)),
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
            .font_weight(FontWeight::MEDIUM)
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

/// A clock's minutes as a time control names them: "½" for 30 seconds, as Lichess writes it,
/// "1.5" for 90.
pub fn clock_minutes(minutes: f64) -> String {
    match minutes {
        0.25 => "¼".into(),
        0.5 => "½".into(),
        0.75 => "¾".into(),
        m if m.fract() == 0. => format!("{m:.0}"),
        m => format!("{}", (m * 100.).round() / 100.),
    }
}

/// A time control, "5+3": minutes on the clock and seconds added each move.
pub fn time_control(minutes: f64, increment: f64) -> String {
    format!("{}+{increment}", clock_minutes(minutes))
}

#[cfg(test)]
mod time_control_tests {
    use super::time_control;

    /// A 30-second clock reads the same on every page: Tournaments, Game history, the
    /// challenges, and a game against the computer.
    #[test]
    fn a_thirty_second_clock_reads_as_lichess_writes_it() {
        assert_eq!(time_control(0.5, 0.), "½+0");
        assert_eq!(time_control(0.25, 1.), "¼+1");
        assert_eq!(time_control(3., 2.), "3+2");
        assert_eq!(time_control(1.5, 0.), "1.5+0");
        assert_eq!(time_control(180., 180.), "180+180");
    }
}
