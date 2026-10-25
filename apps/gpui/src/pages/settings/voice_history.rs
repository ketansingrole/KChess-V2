//! Settings → Voice input → Voice history (`VoiceHistory.vue`): what voice input heard, how it
//! was read and what the player meant. The point is to find what gets misheard: the weakest
//! words, and the phrases that stood for another move.

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::Deserialize;
use serde_json::json;

use crate::assets::icon;
use crate::core::AppCore;
use crate::theme::Palette;
use crate::widgets::{self, Look};

const SUCCESS: [&str; 4] = ["played", "confirmed", "picked", "correct"];
const MISS: [&str; 5] = ["invalid", "unclear", "wrong", "cancelled", "replaced"];
/// Entries read, and shown at first and per Show more.
const LIMIT: usize = 2000;
const SHOWN: usize = 40;
const MORE: usize = 80;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Word {
    pub word: String,
    pub conf: f64,
}

/// One logged phrase (`VoiceAttempt`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Entry {
    pub id: f64,
    pub at: f64,
    pub heard: String,
    pub confidence: f64,
    pub words: Vec<Word>,
    pub outcome: String,
    pub parsed: Option<String>,
    pub expected: Option<String>,
    pub retry_of: Option<f64>,
}

fn outcome_label(outcome: &str) -> &'static str {
    match outcome {
        "played" => "Played",
        "pending" => "Waiting",
        "confirmed" => "Confirmed",
        "picked" => "Picked",
        "cancelled" => "Cancelled",
        "replaced" => "Said again",
        "abandoned" => "Moved by hand",
        "invalid" => "No match",
        "unclear" => "Unclear",
        "command" => "Command",
        "correct" => "Correct",
        "wrong" => "Wrong square",
        _ => "—",
    }
}

fn outcome_look(outcome: &str) -> Look {
    if SUCCESS.contains(&outcome) {
        Look::Success
    } else if matches!(outcome, "unclear" | "cancelled" | "replaced") {
        Look::Warning
    } else if MISS.contains(&outcome) {
        Look::Error
    } else {
        Look::NeutralSoft
    }
}

/// Advice for words that keep scoring low.
fn tip_for(word: &str) -> Option<&'static str> {
    match word {
        "pawn" | "pond" => {
            Some("For pawn moves you can skip the word: just say the square, like “E four”.")
        }
        "b" | "c" | "d" | "e" | "g" => Some(
            "B, C, D, E and G rhyme. The phonetic names (Bravo, Charlie, Delta, Echo, Golf) are much clearer.",
        ),
        "to" | "two" => {
            Some("“To” and “two” sound the same. Leaving “to” out (“E two E four”) works too.")
        }
        _ => None,
    }
}

/// The headline counts.
#[derive(Debug, PartialEq)]
pub struct Stats {
    pub phrases: usize,
    /// Understood, of the phrases that either succeeded or missed (percent).
    pub rate: u32,
    pub repeats: usize,
    pub unclear: usize,
    pub noise: usize,
}

pub fn stats(entries: &[Entry]) -> Stats {
    let counted: Vec<&Entry> = entries
        .iter()
        .filter(|entry| {
            SUCCESS.contains(&entry.outcome.as_str()) || MISS.contains(&entry.outcome.as_str())
        })
        .collect();
    let success = counted
        .iter()
        .filter(|entry| SUCCESS.contains(&entry.outcome.as_str()))
        .count();
    Stats {
        phrases: entries.len(),
        rate: if counted.is_empty() {
            0
        } else {
            (success as f64 / counted.len() as f64 * 100.).round() as u32
        },
        repeats: entries
            .iter()
            .filter(|entry| entry.retry_of.is_some())
            .count(),
        unclear: entries
            .iter()
            .filter(|entry| entry.outcome == "unclear")
            .count(),
        noise: entries
            .iter()
            .filter(|entry| entry.heard.contains("[unk]"))
            .count(),
    }
}

/// Words by average confidence, weakest first; only words heard a few times say anything.
pub fn weak_words(entries: &[Entry]) -> Vec<(String, usize, f64)> {
    let mut by_word: HashMap<&str, (f64, usize)> = HashMap::new();
    for word in entries.iter().flat_map(|entry| &entry.words) {
        let item = by_word.entry(word.word.as_str()).or_default();
        item.0 += word.conf;
        item.1 += 1;
    }
    let mut words: Vec<(String, usize, f64)> = by_word
        .into_iter()
        .map(|(word, (sum, count))| (word.to_string(), count, sum / count as f64))
        .filter(|(_, count, conf)| *count >= 2 && *conf < 0.85)
        .collect();
    words.sort_by(|a, b| a.2.total_cmp(&b.2).then_with(|| a.0.cmp(&b.0)));
    words.truncate(10);
    words
}

/// Phrases that missed, paired with what the player meant, most frequent first.
pub fn mishearings(entries: &[Entry]) -> Vec<(String, String, usize)> {
    let mut groups: Vec<(String, String, usize)> = Vec::new();
    for entry in entries {
        let Some(meant) = &entry.expected else {
            continue;
        };
        if !MISS.contains(&entry.outcome.as_str()) {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(heard, expected, _)| *heard == entry.heard && expected == meant)
        {
            Some(group) => group.2 += 1,
            None => groups.push((entry.heard.clone(), meant.clone(), 1)),
        }
    }
    // Stable: equal counts keep the newest first, as the entries come.
    groups.sort_by_key(|group| std::cmp::Reverse(group.2));
    groups.truncate(10);
    groups
}

fn percent(value: f64) -> String {
    format!("{}%", (value * 100.).round())
}

fn when(at: f64) -> String {
    chrono::DateTime::from_timestamp_millis(at as i64)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

pub struct VoiceHistory {
    entries: Vec<Entry>,
    loading: bool,
    error: Option<SharedString>,
    shown: usize,
}

impl VoiceHistory {
    pub fn new(cx: &mut Context<Self>) -> VoiceHistory {
        let mut history = VoiceHistory {
            entries: Vec::new(),
            loading: true,
            error: None,
            shown: SHOWN,
        };
        history.load(cx);
        history
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let read = cx
            .global::<AppCore>()
            .call("voiceHistory", vec![json!(LIMIT)]);
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match read {
                    Ok(list) => this.entries = serde_json::from_value(list).unwrap_or_default(),
                    Err(cause) => {
                        crate::log::write(
                            kchess_core::Level::Warn,
                            "voice-history",
                            &format!("Could not read voice history: {}", cause.message),
                        );
                        this.error = Some(cause.message.into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let document = cx.global::<AppCore>().call("voiceHistoryDocument", vec![]);
        cx.spawn_in(window, async move |_, cx| {
            let outcome = match document.await {
                Err(cause) => Some(Err(cause.message)),
                Ok(text) => {
                    let name = format!(
                        "kchess-voice-history-{}.json",
                        chrono::Local::now().format("%Y-%m-%d")
                    );
                    let text = text.as_str().unwrap_or_default().to_string();
                    match cx.update(|_, cx| crate::export::save_text(&name, text, cx)) {
                        Ok(saving) => saving.await,
                        Err(_) => None,
                    }
                }
            };
            let _ = cx.update(|window, cx| match outcome {
                Some(Ok(())) => window.push_notification("Saved the voice history", cx),
                Some(Err(cause)) => window
                    .push_notification(Notification::error(cause).title("The export failed"), cx),
                None => {}
            });
        })
        .detach();
    }

    fn ask_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let this = this.clone();
            dialog
                .title("Clear voice history?")
                .description(
                    "Every logged phrase is deleted. Export it first if you want to keep it.",
                )
                .ok_text("Clear")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| this.clear(cx));
                    true
                })
        });
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let clearing = cx.global::<AppCore>().call("clearVoiceHistory", vec![]);
        cx.spawn(async move |this, cx| {
            let cleared = clearing.await;
            let _ = this.update(cx, |this, cx| match cleared {
                Ok(_) => this.load(cx),
                Err(cause) => {
                    crate::log::write(
                        kchess_core::Level::Warn,
                        "voice-history",
                        &format!("Clearing voice history failed: {}", cause.message),
                    );
                    this.error = Some(cause.message.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn subtitle(text: &'static str, palette: &Palette) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette.highlighted)
        .child(text)
}

fn muted(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(12.))
        .text_color(palette.text_muted)
        .child(text.into())
}

impl Render for VoiceHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.entries.is_empty();
        let header = h_flex()
            .justify_between()
            .gap(px(12.))
            .child(widgets::heading("Voice history", &palette))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(
                        Button::new("voice-history-refresh")
                            .ghost()
                            .icon(icon("rotate-cw"))
                            .loading(self.loading)
                            .accessibility_label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    )
                    .child(
                        Button::new("voice-history-export")
                            .outline()
                            .icon(icon("download"))
                            .label("Export JSON")
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
                    )
                    .child(
                        Button::new("voice-history-clear")
                            .ghost()
                            .danger()
                            .icon(icon("trash-2"))
                            .label("Clear")
                            .disabled(empty)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.ask_clear(window, cx)),
                            ),
                    ),
            );
        let mut card = widgets::card(&palette).gap(px(16.)).child(header);
        if let Some(error) = &self.error {
            return card.child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error.clone()),
            );
        }
        if empty {
            if !self.loading {
                card = card.child(muted(
                    "Nothing yet. Speak a few moves in Play with Computer or answer a practice drill by voice, then come back.",
                    &palette,
                ));
            }
            return card;
        }
        let stats = stats(&self.entries);
        let stat = |value: String, label: &'static str| {
            v_flex()
                .flex_1()
                .min_w(px(110.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(palette.elevated)
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::BOLD)
                        .child(value),
                )
                .child(muted(label, &palette))
        };
        let tiles = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(stat(stats.phrases.to_string(), "phrases"))
            .child(stat(format!("{}%", stats.rate), "understood"))
            .child(stat(stats.repeats.to_string(), "had to repeat"))
            .child(stat(stats.unclear.to_string(), "too unclear"))
            .child(stat(stats.noise.to_string(), "with unknown sounds"));
        let weak = weak_words(&self.entries);
        let words = v_flex()
            .flex_1()
            .min_w(px(260.))
            .gap(px(8.))
            .child(subtitle("Least certain words", &palette))
            .when(weak.is_empty(), |column| {
                column.child(muted("No word stands out yet.", &palette))
            })
            .children(weak.into_iter().map(|(word, count, conf)| {
                v_flex()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .text_size(px(13.))
                            .child(
                                div()
                                    .w(px(70.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(word.clone()),
                            )
                            .child(
                                div()
                                    .w(px(100.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(palette.accented)
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_full()
                                            .bg(if conf < 0.6 {
                                                palette.error
                                            } else {
                                                palette.primary
                                            })
                                            .w(relative(conf as f32)),
                                    ),
                            )
                            .child(muted(format!("{} · {count}×", percent(conf)), &palette)),
                    )
                    .children(tip_for(&word).map(|tip| muted(tip, &palette)))
            }));
        let pairs = mishearings(&self.entries);
        let heard_meant = v_flex()
            .flex_1()
            .min_w(px(260.))
            .gap(px(8.))
            .child(subtitle("Heard → meant", &palette))
            .when(pairs.is_empty(), |column| {
                column.child(muted("No corrections yet.", &palette))
            })
            .children(pairs.into_iter().map(|(heard, meant, count)| {
                h_flex()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .child(
                        div()
                            .text_color(palette.text_muted)
                            .child(format!("“{heard}”")),
                    )
                    .child(
                        icon("arrow-right")
                            .size(px(14.))
                            .text_color(palette.text_muted),
                    )
                    .child(div().font_weight(FontWeight::BOLD).child(meant))
                    .when(count > 1, |row| {
                        row.child(muted(format!("{count}×"), &palette))
                    })
            }));
        let column = |width: f32| div().w(px(width)).flex_shrink_0().min_w_0();
        let table_row = |cells: [AnyElement; 5]| {
            let [at, heard, parsed, meant, result] = cells;
            h_flex()
                .gap(px(12.))
                .py(px(6.))
                .border_b_1()
                .border_color(palette.border)
                .text_size(px(13.))
                .child(column(120.).child(at))
                .child(div().flex_1().min_w_0().child(heard))
                .child(column(110.).child(parsed))
                .child(column(70.).child(meant))
                .child(column(110.).child(result))
        };
        let head = |text: &'static str| muted(text, &palette).into_any_element();
        let recent = self.entries.iter().take(self.shown).map(|entry| {
            let words: Vec<AnyElement> = if entry.words.is_empty() {
                vec![div().child(entry.heard.clone()).into_any_element()]
            } else {
                entry
                    .words
                    .iter()
                    .map(|word| {
                        div()
                            .when(word.conf < 0.6, |word| {
                                word.text_color(palette.error).underline()
                            })
                            .child(word.word.clone())
                            .into_any_element()
                    })
                    .collect()
            };
            table_row([
                muted(when(entry.at), &palette).into_any_element(),
                h_flex()
                    .gap(px(4.))
                    .flex_wrap()
                    .when(entry.retry_of.is_some(), |cell| {
                        cell.child(icon("repeat").size(px(13.)).text_color(palette.text_muted))
                    })
                    .children(words)
                    .child(muted(percent(entry.confidence), &palette))
                    .into_any_element(),
                div()
                    .child(
                        entry
                            .parsed
                            .as_deref()
                            .map_or("—".to_string(), |parsed| parsed.replace('|', " / ")),
                    )
                    .into_any_element(),
                div()
                    .child(entry.expected.clone().unwrap_or_else(|| "—".into()))
                    .into_any_element(),
                h_flex()
                    .child(widgets::badge(
                        outcome_label(&entry.outcome),
                        outcome_look(&entry.outcome),
                        &palette,
                    ))
                    .into_any_element(),
            ])
        });
        card.child(tiles)
            .child(
                h_flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(28.))
                    .child(words)
                    .child(heard_meant),
            )
            .child(subtitle("Recent", &palette))
            .child(
                v_flex()
                    .child(table_row([
                        head("When"),
                        head("Heard"),
                        head("Read as"),
                        head("Meant"),
                        head("Result"),
                    ]))
                    .children(recent),
            )
            .when(self.entries.len() > self.shown, |card| {
                card.child(
                    h_flex().child(
                        Button::new("voice-history-more")
                            .ghost()
                            .small()
                            .label("Show more")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.shown += MORE;
                                cx.notify();
                            })),
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Word, mishearings, stats, weak_words};

    fn entry(heard: &str, outcome: &str, expected: Option<&str>, words: &[(&str, f64)]) -> Entry {
        Entry {
            heard: heard.into(),
            outcome: outcome.into(),
            expected: expected.map(String::from),
            words: words
                .iter()
                .map(|(word, conf)| Word {
                    word: (*word).into(),
                    conf: *conf,
                })
                .collect(),
            ..Entry::default()
        }
    }

    #[test]
    fn the_history_finds_weak_words_and_mishearings() {
        let entries = vec![
            entry("e four", "played", Some("e4"), &[("e", 0.5), ("four", 0.9)]),
            entry(
                "b four",
                "invalid",
                Some("e4"),
                &[("b", 0.4), ("four", 0.95)],
            ),
            entry("b four", "wrong", Some("e4"), &[("b", 0.45), ("four", 0.9)]),
            entry("e four [unk]", "unclear", None, &[("e", 0.3)]),
            entry("back", "command", None, &[("back", 0.99)]),
        ];
        let stats = stats(&entries);
        assert_eq!((stats.phrases, stats.unclear, stats.noise), (5, 1, 1));
        // One success among four that either succeeded or missed; commands do not count.
        assert_eq!(stats.rate, 25);
        // “e” averages 0.4 and “b” 0.425; “four” is heard well, and “back” only once.
        let weak = weak_words(&entries);
        assert_eq!(
            weak.iter()
                .map(|(word, ..)| word.as_str())
                .collect::<Vec<_>>(),
            ["e", "b"]
        );
        assert_eq!(mishearings(&entries), [("b four".into(), "e4".into(), 2)]);
    }
}
