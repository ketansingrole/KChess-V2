//! Settings → Data & storage: what KChess downloaded from Lichess for each
//! account and keeps on this computer, clearing the local scores, and the diagnostics export.

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{Section, SettingsPage};
use crate::assets::icon;
use crate::core::AppCore;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Look, format_bytes, format_count};

/// One account's line: what it keeps here and what it downloaded.
#[derive(Clone, Debug, PartialEq)]
pub struct DataRow {
    pub label: String,
    pub badge: Option<&'static str>,
    pub detail: Option<&'static str>,
    pub stored: f64,
    pub games: f64,
    pub downloaded: f64,
    pub requests: f64,
}

/// One row per account with anything to report, most data first; removed accounts stay listed
/// so past downloads are not hidden, and requests tied to no account have their own row.
pub fn data_rows(accounts: &[Value], usage: &Value) -> Vec<DataRow> {
    let known = |name: &str| {
        accounts.iter().find(|account| {
            account["username"]
                .as_str()
                .is_some_and(|username| username.to_lowercase() == name)
        })
    };
    let mut names: Vec<String> = accounts
        .iter()
        .filter_map(|account| account["username"].as_str().map(str::to_lowercase))
        .collect();
    for key in ["accounts", "storage"] {
        for name in usage[key]
            .as_object()
            .into_iter()
            .flat_map(|map| map.keys())
        {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    let number = |value: &Value| value.as_f64().unwrap_or_default();
    let mut rows: Vec<(bool, DataRow)> = names
        .into_iter()
        .map(|name| {
            let account = known(&name);
            let used = &usage["accounts"][&name]["total"];
            let kept = &usage["storage"][&name];
            let label = if name.is_empty() {
                "App (no account)".to_string()
            } else {
                format!(
                    "@{}",
                    account
                        .and_then(|account| account["username"].as_str())
                        .unwrap_or(&name)
                )
            };
            let badge = match account {
                Some(account) if account["connected"] == json!(true) => Some("Connected"),
                Some(_) => Some("Friend"),
                None if !name.is_empty() => Some("Removed"),
                None => None,
            };
            (
                account.is_some(),
                DataRow {
                    label,
                    badge,
                    detail: name
                        .is_empty()
                        .then_some("Sign-in and other requests not tied to one account"),
                    stored: number(&kept["bytes"]) + number(&kept["cacheBytes"]),
                    games: number(&kept["games"]),
                    downloaded: number(&used["bytesIn"]),
                    requests: number(&used["requests"]),
                },
            )
        })
        .filter(|(known, row)| {
            *known || row.stored > 0. || row.downloaded > 0. || row.requests > 0.
        })
        .collect();
    rows.sort_by(|(_, a), (_, b)| (b.downloaded + b.stored).total_cmp(&(a.downloaded + a.stored)));
    rows.into_iter().map(|(_, row)| row).collect()
}

/// Every account's total of one of `usage.accounts`' numbers, or of `usage.storage`'s.
fn total(usage: &Value, key: &str, fields: &[&str]) -> f64 {
    usage[key]
        .as_object()
        .into_iter()
        .flat_map(|map| map.values())
        .map(|entry| {
            fields
                .iter()
                .map(|field| {
                    field
                        .split('.')
                        .fold(entry, |value, part| &value[part])
                        .as_f64()
                        .unwrap_or_default()
                })
                .sum::<f64>()
        })
        .sum()
}

impl SettingsPage {
    fn ask_reset_usage(window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, |dialog: AlertDialog, _, _| {
            dialog
                .title("Reset the data counters?")
                .description("This only clears the download counts shown here. Your stored games and accounts are not touched.")
                .ok_text("Reset")
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(|_, _, cx| {
                    let reset = cx.global::<AppCore>().call("resetUsage", vec![]);
                    cx.spawn(async move |cx| {
                        if let Err(cause) = reset.await {
                            crate::log::write(
                                kchess_core::Level::Warn,
                                "usage",
                                &format!("Resetting usage failed: {}", cause.message),
                            );
                        }
                        cx.update(|cx| {
                            crate::accounts::handle(cx)
                                .update(cx, |accounts, cx| accounts.refresh(cx))
                        });
                    })
                    .detach();
                    true
                })
        });
    }

    fn ask_clear_runs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let this = this.clone();
            dialog
                .title("Clear your local scores?")
                .description("This deletes the Storm, Streak, Rush and board-training scores and the endgame and lesson progress stored on this computer. Nothing on Lichess is affected.")
                .ok_text("Clear")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| this.clear_runs(cx));
                    true
                })
        });
    }

    fn clear_runs(&mut self, cx: &mut Context<Self>) {
        self.runs_note = None;
        let clearing = cx.global::<AppCore>().call("clearRuns", vec![]);
        cx.spawn(async move |this, cx| {
            let cleared = clearing.await;
            let _ = this.update(cx, |this, cx| {
                this.runs_note = Some(match cleared {
                    Ok(_) => "Local scores and progress cleared".into(),
                    Err(cause) => {
                        crate::log::write(
                            kchess_core::Level::Warn,
                            "settings",
                            &format!("Clearing local scores failed: {}", cause.message),
                        );
                        cause.message.into()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// The redacted logs and this build's facts, saved where the player chooses
    /// (`exportDiagnostics`).
    fn export_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.exporting = true;
        self.diagnostics_note = None;
        let report = json!({
            "generatedAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "sessionId": crate::log::session_id(),
            "version": crate::updates::current_version(),
            "client": "gpui",
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "versions": { "app": "gpui" },
            "logs": crate::log::read(),
        });
        let name = format!(
            "kchess-diagnostics-{}.json",
            chrono::Local::now().format("%Y-%m-%d")
        );
        let text = serde_json::to_string_pretty(&report).unwrap_or_default();
        let saving = crate::export::save_text(&name, text, cx);
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let _ = this.update(cx, |this, cx| {
                this.exporting = false;
                this.diagnostics_note = match saved {
                    Some(Ok(())) => Some("Diagnostics exported".into()),
                    Some(Err(cause)) => {
                        crate::log::write(
                            kchess_core::Level::Warn,
                            "settings",
                            &format!("Diagnostics export failed: {cause}"),
                        );
                        Some(cause.into())
                    }
                    None => None,
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn data(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let (rows, usage) = {
            let accounts = crate::accounts::handle(cx);
            let accounts = accounts.read(cx);
            (
                data_rows(&accounts.all(), &accounts.usage),
                accounts.usage.clone(),
            )
        };
        let downloaded = total(&usage, "accounts", &["total.bytesIn"]);
        let requests = total(&usage, "accounts", &["total.requests"]);
        let stored = total(&usage, "storage", &["bytes", "cacheBytes"]);
        let since = usage["since"]
            .as_f64()
            .and_then(|millis| chrono::DateTime::from_timestamp_millis(millis as i64))
            .map(|date| {
                format!(
                    " since {}",
                    date.with_timezone(&chrono::Local).format("%-m/%-d/%Y")
                )
            })
            .unwrap_or_default();
        let muted = |text: String| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(text)
        };
        // `.data-totals > div`: a bordered tile, at least 200px.
        let tile = |label: &'static str, value: String, note: Option<String>| {
            v_flex()
                .flex_1()
                .min_w(px(200.))
                .gap(px(2.))
                .px(px(16.))
                .py(px(14.))
                .rounded(radii::LG)
                .border_1()
                .border_color(palette.border)
                .bg(palette.bg)
                .child(muted(label.into()))
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(value),
                )
                .children(note.map(muted))
        };
        let number_cell = |child: AnyElement| {
            v_flex()
                .w(px(130.))
                .flex_shrink_0()
                .items_end()
                .child(child)
        };
        let table_row = |account: AnyElement, cells: [AnyElement; 3]| {
            let [kept, down, count] = cells;
            h_flex()
                .gap(px(12.))
                .py(px(8.))
                .border_b_1()
                .border_color(palette.border)
                .text_size(px(13.))
                .child(div().flex_1().min_w_0().child(account))
                .child(number_cell(kept))
                .child(number_cell(down))
                .child(number_cell(count))
        };
        let head = |text: &'static str| muted(text.into()).into_any_element();
        let usage_card = widgets::card(palette)
            .gap(px(16.))
            .child(
                h_flex().justify_end().child(
                    Button::new("data-reset-usage")
                        .outline()
                        .icon(icon("rotate-ccw"))
                        .label("Reset counters")
                        .disabled(requests == 0.)
                        .on_click(|_, window, cx| Self::ask_reset_usage(window, cx)),
                ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .items_stretch()
                    .gap(px(12.))
                    .child(tile(
                        "Downloaded",
                        format_bytes(downloaded),
                        Some(format!("{} requests{since}", format_count(requests))),
                    ))
                    .child(tile("Stored games & profiles", format_bytes(stored), None))
                    .child(tile(
                        "Database file",
                        format_bytes(usage["dbBytes"].as_f64().unwrap_or_default()),
                        None,
                    )),
            )
            .map(|card| {
                if rows.is_empty() {
                    return card.child(widgets::empty_state(
                        icon("database"),
                        "Nothing counted yet",
                        None,
                        palette,
                    ));
                }
                card.child(
                    v_flex()
                        .child(table_row(
                            head("Account"),
                            [head("Stored here"), head("Downloaded"), head("Requests")],
                        ))
                        .children(rows.into_iter().map(|row| {
                            table_row(
                                v_flex()
                                    .child(
                                        h_flex()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(row.label),
                                            )
                                            .children(row.badge.map(|badge| {
                                                widgets::badge(badge, Look::NeutralSoft, palette)
                                            })),
                                    )
                                    .children(row.detail.map(|detail| muted(detail.into())))
                                    .into_any_element(),
                                [
                                    v_flex()
                                        .items_end()
                                        .child(format_bytes(row.stored))
                                        .child(muted(format!("{} games", format_count(row.games))))
                                        .into_any_element(),
                                    div().child(format_bytes(row.downloaded)).into_any_element(),
                                    div().child(format_count(row.requests)).into_any_element(),
                                ],
                            )
                        })),
                )
            });
        let puzzles = widgets::card(palette)
            .gap(px(12.))
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .child(widgets::heading("Puzzles & practice", palette))
                    .child(
                        Button::new("data-clear-runs")
                            .outline()
                            .icon(icon("eraser"))
                            .label("Clear local scores")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.ask_clear_runs(window, cx)),
                            ),
                    ),
            )
            .child(
                h_flex().child(
                    Button::new("data-offline")
                        .outline()
                        .icon(icon("download"))
                        .label("Offline downloads")
                        .on_click(|_, window, cx| {
                            crate::shell::open_settings(Section::Offline, window, cx)
                        }),
                ),
            )
            .children(
                self.runs_note
                    .clone()
                    .map(|note| widgets::hint(note, palette)),
            );
        let diagnostics = super::Rows::new(self.narrow())
            .row(
                "Diagnostics",
                Some(self.diagnostics_note.clone().unwrap_or_else(|| {
                    "Exports logs without tokens, games or account databases".into()
                })),
                h_flex().child(
                    Button::new("data-diagnostics")
                        .outline()
                        .label("Export diagnostics")
                        .loading(self.exporting)
                        .on_click(cx.listener(|this, _, _, cx| this.export_diagnostics(cx))),
                ),
                palette,
            )
            .card(palette);
        v_flex()
            .gap(px(16.))
            .child(usage_card)
            .child(puzzles)
            .child(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::data_rows;
    use serde_json::json;

    #[test]
    fn every_account_with_data_gets_a_row_most_first() {
        let accounts = vec![
            json!({ "username": "Me", "connected": true }),
            json!({ "username": "Friend" }),
        ];
        let usage = json!({
            "accounts": {
                "me": { "total": { "bytesIn": 100, "requests": 3 } },
                "gone": { "total": { "bytesIn": 5000, "requests": 1 } },
                "": { "total": { "bytesIn": 10, "requests": 2 } },
                "quiet": { "total": { "bytesIn": 0, "requests": 0 } },
            },
            "storage": { "friend": { "games": 4, "bytes": 300, "cacheBytes": 20 } },
        });
        let rows = data_rows(&accounts, &usage);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["@gone", "@Friend", "@Me", "App (no account)"]);
        assert_eq!(rows[0].badge, Some("Removed"));
        assert_eq!(
            (rows[1].badge, rows[1].stored, rows[1].games),
            (Some("Friend"), 320., 4.)
        );
        assert_eq!(rows[2].badge, Some("Connected"));
        assert!(rows[3].detail.is_some());
    }
}
