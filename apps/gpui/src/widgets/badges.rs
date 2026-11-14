//! Badges, result rows, notifications and counts as the app writes them.

use super::*;

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

/// "100,000".
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

/// A count with its noun: "1 request", "2,048 requests".
pub fn format_counted(count: f64, one: &str, many: &str) -> String {
    let shown = format_count(count);
    format!("{shown} {}", if shown == "1" { one } else { many })
}

/// "312 MB", "1.5 GB".
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

/// "just now", "5 min ago", "3 h ago", "2 days ago", else the date.
pub fn time_ago(timestamp: f64, now: f64) -> String {
    if timestamp <= 0. {
        return "never".into();
    }
    let seconds = ((now - timestamp) / 1000.).round().max(0.);
    if seconds < 45. {
        return "just now".into();
    }
    let minutes = (seconds / 60.).round();
    if minutes < 60. {
        return format!("{minutes} min ago");
    }
    let hours = (minutes / 60.).round();
    if hours < 24. {
        return format!("{hours} h ago");
    }
    let days = (hours / 24.).round();
    if days < 31. {
        return format!("{days} day{} ago", if days == 1. { "" } else { "s" });
    }
    // The numeric date, 12/30/2025.
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(timestamp as i64)
        .single()
        .map(|date| date.format("%-m/%-d/%Y").to_string())
        .unwrap_or_default()
}

/// How a badge is drawn: its color and fill.
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

/// A short label in a soft color; callers add an icon, a width or a dot.
pub fn badge(label: impl Into<SharedString>, look: Look, palette: &Palette) -> Div {
    badge_box(look, palette).child(label.into())
}

/// A badge with its leading icon (`UBadge :icon`).
pub fn icon_badge(
    glyph: Icon,
    label: impl Into<SharedString>,
    look: Look,
    palette: &Palette,
) -> Div {
    badge_box(look, palette)
        .child(glyph.size(px(13.)))
        .child(label.into())
}

fn badge_box(look: Look, palette: &Palette) -> Div {
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
        .gap(px(8.))
        .px(px(8.))
        .py(px(4.))
        .rounded(radii::SM)
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .when_some(ring, |badge, ring| badge.border_1().border_color(ring))
}

/// A label, the score (wins plus half the draws), the games, and a bar of won,
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
        .py(px(4.))
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
                    // The score at the row's weight, its unit 12px and muted.
                    h_flex()
                        .gap(px(4.))
                        .items_baseline()
                        .child(if total > 0. {
                            format!("{score}%")
                        } else {
                            "–".into()
                        })
                        .when(total > 0., |value| {
                            value.child(
                                div()
                                    .text_size(px(12.))
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
                .rounded(radii::XS)
                .overflow_hidden()
                .bg(palette.elevated)
                .when(total > 0., |bar| {
                    bar.tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                })
                .children({
                    let shown: Vec<_> = parts.into_iter().filter(|(part, _)| *part > 0.).collect();
                    let count = shown.len();
                    shown
                        .into_iter()
                        .enumerate()
                        .map(move |(index, (part, color))| {
                            bar_segment(div(), (index == 0, index + 1 == count), radii::XS)
                                .h_full()
                                .w(relative((part / total) as f32))
                                .bg(color)
                        })
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::time_ago;

    #[test]
    fn edits_read_as_how_long_ago() {
        let now = 1_000_000_000_000.;
        assert_eq!(time_ago(now - 10_000., now), "just now");
        assert_eq!(time_ago(now - 5. * 60_000., now), "5 min ago");
        assert_eq!(time_ago(now - 3. * 3_600_000., now), "3 h ago");
        assert_eq!(time_ago(now - 86_400_000., now), "1 day ago");
        assert_eq!(time_ago(0., now), "never");
        // Older than a month: the numeric date.
        let old = time_ago(now - 90. * 86_400_000., now);
        assert_eq!(old.matches('/').count(), 2, "{old}");
    }
}
