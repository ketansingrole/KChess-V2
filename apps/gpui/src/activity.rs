//! Lines of Lichess activity by day: a player's activity on Players, and the Following list's
//! timeline on Following.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{ActivityDay, ActivityItem, TimelineEntry};

use crate::assets::icon;
use crate::pages::players::player_link;
use crate::theme::Palette;
use crate::widgets;

/// One line (`ActivityItem`) and, on a timeline, the player it is about.
pub struct ActivityRow {
    pub player: Option<String>,
    pub item: ActivityItem,
}

/// One day and its lines.
pub struct ActivityGroup {
    pub label: String,
    pub rows: Vec<ActivityRow>,
}

/// The day as people say it: today, yesterday, a weekday this week, then the date (with its year
/// when it is not this one).
fn day_label(millis: f64, today: chrono::NaiveDate) -> String {
    use chrono::{Datelike as _, TimeZone as _};
    let Some(day) = chrono::Local.timestamp_millis_opt(millis as i64).single() else {
        return String::new();
    };
    let date = day.date_naive();
    match (today - date).num_days() {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => day.format("%A").to_string(),
        _ if date.year() == today.year() => day.format("%-d %b").to_string(),
        _ => day.format("%-d %b %Y").to_string(),
    }
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// `activityGroups`: a player's activity (`ActivityDay`s), one heading per day.
pub fn activity_groups(days: &[ActivityDay]) -> Vec<ActivityGroup> {
    let today = today();
    days.iter()
        .map(|day| ActivityGroup {
            label: day_label(day.start, today),
            rows: day
                .items
                .iter()
                .map(|item| ActivityRow {
                    player: None,
                    item: item.clone(),
                })
                .collect(),
        })
        .collect()
}

/// `timelineGroups`: timeline entries (newest first) under the day they happened.
pub fn timeline_groups(entries: &[TimelineEntry]) -> Vec<ActivityGroup> {
    timeline_groups_on(entries, today())
}

fn timeline_groups_on(entries: &[TimelineEntry], today: chrono::NaiveDate) -> Vec<ActivityGroup> {
    let mut groups: Vec<ActivityGroup> = Vec::new();
    for entry in entries {
        let label = day_label(entry.date, today);
        let row = ActivityRow {
            player: Some(entry.player.clone()),
            item: entry.item.clone(),
        };
        match groups.last_mut() {
            Some(last) if last.label == label => last.rows.push(row),
            _ => groups.push(ActivityGroup {
                label,
                rows: vec![row],
            }),
        }
    }
    groups
}

fn glyph(kind: &str) -> &'static str {
    match kind {
        "games" => "swords",
        "puzzles" => "puzzle",
        "tournament" => "trophy",
        "follow" => "user-plus",
        "study" => "book-open",
        "team" => "users",
        "post" => "message-square",
        "simul" => "layout-grid",
        "patron" => "heart",
        "stream" => "radio",
        "correspondence" => "mail",
        _ => "dot",
    }
}

/// What happened, a name in it a profile link.
fn line(id: (&'static str, usize), item: &ActivityItem, palette: &Palette) -> Div {
    let text = item.text.clone();
    let named = item
        .username
        .as_deref()
        .and_then(|name| text.find(name).map(|at| (name.to_string(), at)));
    let row = h_flex().flex_1().min_w_0().gap(px(8.)).items_start().child(
        icon(glyph(&item.kind))
            .size(px(14.))
            .mt(px(2.))
            .flex_none()
            .text_color(palette.text_muted),
    );
    match named {
        Some((name, at)) => row.child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .child(text[..at].to_string())
                .child(
                    player_link(id, name.clone(), palette.primary)
                        .text_color(palette.primary)
                        .test_support(),
                )
                .child(text[at + name.len()..].to_string()),
        ),
        None => row.child(div().flex_1().min_w_0().child(text)),
    }
}

/// The rating a line ended at and how far it moved, in columns as on the leaderboard.
fn rating(item: &ActivityItem, palette: &Palette) -> Div {
    let cells = h_flex()
        .w(px(104.))
        .flex_none()
        .justify_end()
        .gap(px(8.))
        .font_features(widgets::tabular());
    let Some(change) = item.rating else {
        return cells;
    };
    let (before, after) = (change.before, change.after);
    let moved = after - before;
    cells.child(format!("{after}")).child(
        div()
            .w(px(36.))
            .text_right()
            .text_size(px(12.))
            .text_color(if moved > 0. {
                palette.success
            } else if moved < 0. {
                palette.error
            } else {
                palette.text_muted
            })
            .child(match moved {
                moved if moved > 0. => format!("+{moved}"),
                moved if moved < 0. => format!("{moved}"),
                _ => "±0".to_string(),
            }),
    )
}

/// A player's picture and name, a profile link.
fn player(id: ElementId, name: &str, palette: &Palette) -> Div {
    h_flex()
        .min_w_0()
        .gap(px(8.))
        .child(widgets::initials(name, 22., palette))
        .child(
            div().min_w_0().child(
                player_link(id, name.to_string(), palette.primary)
                    .font_weight(FontWeight::MEDIUM)
                    .test_support(),
            ),
        )
}

/// The groups as a table: the day once in its column, on a timeline the player once a day beside
/// their lines, what happened, and the rating it moved. When `narrow`, the day heads its lines and
/// each player heads theirs, so the text keeps the width.
pub fn activity_list(
    id: &'static str,
    groups: &[ActivityGroup],
    narrow: bool,
    palette: &Palette,
) -> Div {
    let players = groups
        .iter()
        .any(|group| group.rows.iter().any(|row| row.player.is_some()));
    let mut index = 0;
    v_flex()
        .text_size(px(13.))
        .children(groups.iter().enumerate().map(|(at, group)| {
            let day = div()
                .id(ElementId::NamedInteger(
                    format!("{id}-day").into(),
                    at as u64,
                ))
                .test_support()
                .flex_none()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(group.label.clone());
            let mut shown: Option<&str> = None;
            let mut rows = Vec::new();
            for row in &group.rows {
                index += 1;
                // A player's lines of the day follow each other: their name heads the first.
                let first = row.player.as_deref() != shown;
                shown = row.player.as_deref();
                let name = row.player.as_deref().filter(|_| first);
                let link = ElementId::NamedInteger(format!("{id}-player").into(), index as u64);
                if narrow && let Some(name) = name {
                    rows.push(
                        player(link.clone(), name, palette)
                            .px(px(8.))
                            .pt(px(4.))
                            .into_any_element(),
                    );
                }
                rows.push(
                    h_flex()
                        .id(ElementId::NamedInteger(
                            format!("{id}-row").into(),
                            index as u64,
                        ))
                        .gap(px(16.))
                        .items_start()
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(crate::theme::radii::MD)
                        .hover(|line| line.bg(palette.accented))
                        .when(players && !narrow, |line| {
                            line.child(
                                div()
                                    .w(px(168.))
                                    .flex_none()
                                    .min_w_0()
                                    .when_some(name, |cell, name| {
                                        cell.child(player(link, name, palette))
                                    }),
                            )
                        })
                        // Under its player's name, past the picture.
                        .when(players && narrow, |line| line.ml(px(22.)))
                        .child(line((id, index), &row.item, palette))
                        .child(rating(&row.item, palette))
                        .into_any_element(),
                );
            }
            let lines = v_flex().flex_1().min_w_0().gap(px(2.)).children(rows);
            let group_box = if narrow {
                v_flex().gap(px(6.)).child(day.px(px(8.))).child(lines)
            } else {
                h_flex()
                    .gap(px(8.))
                    .items_start()
                    // On the lines' first baseline.
                    .child(day.w(px(88.)).pt(px(5.)).pl(px(8.)))
                    .child(lines)
            };
            group_box
                .py(px(8.))
                .when(at > 0, |day| day.border_t_1().border_color(palette.border))
        }))
}

#[cfg(test)]
mod tests {
    use super::{day_label, timeline_groups_on};
    use kchess_contracts::types::TimelineEntry;
    use serde_json::json;

    #[test]
    fn timeline_entries_of_one_day_share_a_heading() {
        let day = 1_761_600_000_000.0;
        let entries: Vec<TimelineEntry> = serde_json::from_value(json!([
            { "date": day + 60_000.0, "player": "Ann", "item": { "kind": "follow", "text": "a" } },
            { "date": day, "player": "Bob", "item": { "kind": "games", "text": "b" } },
            { "date": day - 3.0 * 86_400_000.0, "player": "Cal", "item": { "kind": "games", "text": "c" } },
        ]))
        .unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let groups = timeline_groups_on(&entries, today);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].rows.len(), 2);
        assert_eq!(groups[0].rows[1].player.as_deref(), Some("Bob"));
    }

    #[test]
    fn days_read_as_people_say_them() {
        use chrono::TimeZone as _;
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let at = |y, m, d| {
            chrono::Local
                .with_ymd_and_hms(y, m, d, 12, 0, 0)
                .unwrap()
                .timestamp_millis() as f64
        };
        assert_eq!(day_label(at(2026, 11, 3), today), "Today");
        assert_eq!(day_label(at(2026, 11, 2), today), "Yesterday");
        assert_eq!(day_label(at(2026, 10, 30), today), "Friday");
        assert_eq!(day_label(at(2026, 10, 20), today), "20 Oct");
        assert_eq!(day_label(at(2025, 12, 24), today), "24 Dec 2025");
    }
}
