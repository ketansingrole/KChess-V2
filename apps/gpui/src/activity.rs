//! Lines of Lichess activity under day headings: a
//! player's activity on Players, and the online account's timeline on Following.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use serde_json::Value;

use crate::assets::icon;
use crate::pages::players::player_link;
use crate::theme::Palette;

/// One heading and its lines (`ActivityItem`s).
pub struct ActivityGroup {
    pub label: String,
    pub items: Vec<Value>,
}

fn day_label(millis: f64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|day| day.format("%a %-d %b").to_string())
        .unwrap_or_default()
}

/// `activityGroups`: a player's activity (`ActivityDay`s), one heading per day.
pub fn activity_groups(days: &[Value]) -> Vec<ActivityGroup> {
    days.iter()
        .map(|day| ActivityGroup {
            label: day_label(day["start"].as_f64().unwrap_or_default()),
            items: day["items"].as_array().cloned().unwrap_or_default(),
        })
        .collect()
}

/// `timelineGroups`: timeline entries (newest first) under the day they happened.
pub fn timeline_groups(entries: &[Value]) -> Vec<ActivityGroup> {
    let mut groups: Vec<ActivityGroup> = Vec::new();
    for entry in entries {
        let label = day_label(entry["date"].as_f64().unwrap_or_default());
        match groups.last_mut() {
            Some(last) if last.label == label => last.items.push(entry["item"].clone()),
            _ => groups.push(ActivityGroup {
                label,
                items: vec![entry["item"].clone()],
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

/// One line, its player's name a profile link.
fn line(id: (&'static str, usize), item: &Value, palette: &Palette) -> Div {
    let text = item["text"].as_str().unwrap_or_default().to_string();
    let named = item["username"]
        .as_str()
        .and_then(|name| text.find(name).map(|at| (name.to_string(), at)));
    let row = h_flex().gap(px(8.)).items_start().child(
        icon(glyph(item["kind"].as_str().unwrap_or_default()))
            .size(px(14.))
            .mt(px(2.))
            .flex_none()
            .text_color(palette.text_muted),
    );
    match named {
        Some((name, at)) => row.child(
            h_flex()
                .flex_wrap()
                .child(text[..at].to_string())
                .child(player_link(id, name.clone(), palette.primary).text_color(palette.primary))
                .child(text[at + name.len()..].to_string()),
        ),
        None => row.child(text),
    }
}

/// The groups as headed lists.
pub fn activity_list(id: &'static str, groups: &[ActivityGroup], palette: &Palette) -> Div {
    let mut index = 0;
    v_flex()
        .gap(px(14.))
        .text_size(px(13.))
        .children(groups.iter().map(|group| {
            v_flex()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(group.label.clone()),
                )
                .children(group.items.iter().map(|item| {
                    index += 1;
                    line((id, index), item, palette)
                }))
        }))
}

#[cfg(test)]
mod tests {
    use super::timeline_groups;
    use serde_json::json;

    #[test]
    fn timeline_entries_of_one_day_share_a_heading() {
        let day = 1_761_600_000_000.0;
        let entries = [
            json!({ "date": day + 60_000.0, "item": { "kind": "follow", "text": "a" } }),
            json!({ "date": day, "item": { "kind": "games", "text": "b" } }),
            json!({ "date": day - 3.0 * 86_400_000.0, "item": { "kind": "games", "text": "c" } }),
        ];
        let groups = timeline_groups(&entries);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].items.len(), 2);
    }
}
