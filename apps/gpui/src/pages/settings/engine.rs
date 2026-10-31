//! Settings → Chess engine: which
//! Stockfish is in use (the bundled one, the one KChess downloaded, or a file chosen), downloading
//! and deleting it, and the background game review.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{EngineStatus, ReviewAuto, Settings};
use serde_json::{Value, json};

use super::{Rows, SettingsPage, choice, info};
use crate::assets::icon;
use crate::core::AppCore;
use crate::reviews::ReviewHandle;
use crate::theme::Palette;
use crate::widgets::{self, Look};

/// Which Stockfish is in use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Bundled,
    Downloaded,
    Custom,
}

/// `engineSource`: the bundled one (no path), the one KChess downloaded, or a chosen file.
fn source(settings: &Settings, status: Option<&EngineStatus>) -> Source {
    let path = &settings.core.engine_path;
    if path.is_empty() {
        Source::Bundled
    } else if status.is_some_and(|status| *path == status.managed.path) {
        Source::Downloaded
    } else {
        Source::Custom
    }
}

/// `engineName`: which Stockfish is in use, in words; empty until it is found ready.
pub fn engine_name(status: Option<&EngineStatus>) -> String {
    let Some(status) = status.filter(|status| status.ready) else {
        return String::new();
    };
    if status.bundled {
        "Bundled Stockfish".into()
    } else if status.path == status.managed.path {
        match &status.managed.version {
            Some(version) => format!("Downloaded Stockfish {version}"),
            None => "Downloaded Stockfish".into(),
        }
    } else {
        "Your Stockfish".into()
    }
}

/// `reviewStatusText`: what the background review is doing.
fn review_text(status: &Value, auto: ReviewAuto) -> String {
    let waiting = status["waiting"].as_u64().unwrap_or_default();
    let current = &status["current"];
    if current.is_object() {
        let more = if waiting > 0 {
            format!(" · {waiting} more waiting")
        } else {
            String::new()
        };
        return format!(
            "Reviewing a game: {} of {} positions{more}.",
            current["done"].as_u64().unwrap_or_default(),
            current["total"].as_u64().unwrap_or_default()
        );
    }
    match status["paused"].as_str() {
        Some("battery") => "Paused while on battery power".into(),
        Some("engine") => "Paused while the engine is in use".into(),
        Some("online") => "Paused during your online game".into(),
        Some("off") => "Games are reviewed when you ask for it".into(),
        _ if auto == ReviewAuto::Off => "Games are reviewed when you ask for it".into(),
        _ if waiting > 0 => format!("{waiting} games waiting"),
        _ => "All caught up".into(),
    }
}

impl SettingsPage {
    /// Ask the core which engine it finds (`refreshEngine`).
    pub(super) fn refresh_engine(&mut self, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().call("engineStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let status = status.await;
            let _ = this.update(cx, |this, cx| {
                this.engine = status
                    .ok()
                    .and_then(|status| serde_json::from_value(status).ok());
                cx.notify();
            });
        })
        .detach();
    }

    /// Set the engine path, then check the engine once the core has it; `chosen` says when a
    /// chosen file did not start as an engine.
    fn set_engine_path(
        &mut self,
        path: String,
        chosen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::settings::change(cx, move |settings| settings.core.engine_path = path);
        let saved = crate::settings::flush(cx);
        cx.spawn_in(window, async move |this, cx| {
            let _ = saved.await;
            let status = this
                .update(cx, |_, cx| {
                    cx.global::<AppCore>().call("engineStatus", Vec::new())
                })
                .ok();
            let Some(status) = status else { return };
            let status: Option<EngineStatus> = status
                .await
                .ok()
                .and_then(|status| serde_json::from_value(status).ok());
            let _ = this.update_in(cx, |this, window, cx| {
                let ready = status.as_ref().is_some_and(|status| status.ready);
                this.engine = status;
                if chosen && !ready {
                    window.push_notification(
                        "That file did not start as a chess engine. Choose a UCI engine such as \
                         Stockfish.",
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `chooseEngine`: any file may be picked; the core trusts it as an engine path.
    fn choose_engine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.first().map(|path| path.to_string_lossy().to_string()) else {
                return;
            };
            let trusted = this
                .update(cx, |_, cx| {
                    cx.global::<AppCore>()
                        .call("trustEnginePath", vec![json!(path)])
                })
                .ok();
            if let Some(trusted) = trusted {
                let _ = trusted.await;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_engine_path(path, true, window, cx)
            });
        })
        .detach();
    }

    /// Check for the latest native Stockfish and download it if needed. A first download becomes
    /// the engine in use; updating an existing copy leaves the current choice alone.
    fn install_engine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.engine_busy {
            return;
        }
        let had_download = self
            .engine
            .as_ref()
            .is_some_and(|status| status.managed.installed);
        self.engine_busy = true;
        cx.notify();
        let install = cx.global::<AppCore>().call("installEngine", Vec::new());
        cx.spawn_in(window, async move |this, cx| {
            let result = install.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.engine_busy = false;
                let result = match result {
                    Ok(result) => result,
                    Err(cause) => {
                        window.push_notification(
                            Notification::error(cause.message)
                                .title("Could not download Stockfish"),
                            cx,
                        );
                        cx.notify();
                        return;
                    }
                };
                let version = result["version"].as_str().unwrap_or_default().to_string();
                if result["updated"] != json!(true) {
                    window.push_notification(
                        SharedString::from(format!(
                            "You already have the latest Stockfish ({version})"
                        )),
                        cx,
                    );
                    this.refresh_engine(cx);
                } else if had_download {
                    window.push_notification(
                        SharedString::from(format!("Stockfish updated to {version}")),
                        cx,
                    );
                    this.refresh_engine(cx);
                } else {
                    window.push_notification(
                        SharedString::from(format!("Stockfish {version} downloaded")),
                        cx,
                    );
                    let path = result["path"].as_str().unwrap_or_default().to_string();
                    this.set_engine_path(path, false, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_delete_engine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let page = page.clone();
            dialog
                .title("Delete downloaded Stockfish?")
                .description(
                    "This removes the copy KChess downloaded. The bundled engine and any \
                     executable you chose yourself are not touched, and you can download it again \
                     later.",
                )
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let _ = page.update(cx, |page, cx| page.delete_engine(window, cx));
                    true
                })
        });
    }

    /// Delete the downloaded engine; when it was in use, the bundled one takes over.
    fn delete_engine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let managed = self
            .engine
            .as_ref()
            .map(|status| status.managed.path.clone())
            .unwrap_or_default();
        self.engine_busy = true;
        cx.notify();
        let delete = cx.global::<AppCore>().call("deleteEngine", Vec::new());
        cx.spawn_in(window, async move |this, cx| {
            let result = delete.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.engine_busy = false;
                if let Err(cause) = result {
                    window.push_notification(
                        Notification::error(cause.message).title("Could not delete Stockfish"),
                        cx,
                    );
                    cx.notify();
                    return;
                }
                let in_use = cx
                    .global::<crate::settings::SettingsHandle>()
                    .0
                    .read(cx)
                    .settings
                    .core
                    .engine_path
                    == managed;
                if in_use {
                    this.set_engine_path(String::new(), false, window, cx);
                } else {
                    this.refresh_engine(cx);
                }
                window.push_notification("Downloaded Stockfish deleted", cx);
            });
        })
        .detach();
    }

    /// One engine, whether it is in use, and its buttons.
    fn engine_option(
        &self,
        title: &'static str,
        description: String,
        active: bool,
        buttons: Vec<AnyElement>,
        palette: &Palette,
    ) -> AnyElement {
        let head = v_flex()
            .min_w_0()
            .gap(px(4.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.highlighted)
                            .truncate()
                            .child(title),
                    )
                    .when(active, |head| {
                        head.child(widgets::badge("In use", Look::Primary, palette))
                    }),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(description),
            );
        let controls = h_flex().gap(px(8.)).flex_none().children(buttons);
        if self.narrow() {
            v_flex()
                .gap(px(8.))
                .child(head)
                .child(controls)
                .into_any_element()
        } else {
            h_flex()
                .gap(px(40.))
                .child(head.w(px(420.)).min_w(px(200.)).flex_shrink(1.))
                .child(div().flex_1().min_w_0().child(controls))
                .into_any_element()
        }
    }

    pub(super) fn engine(
        &self,
        settings: &Settings,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let status = self.engine.as_ref();
        let ready = status.is_some_and(|status| status.ready);
        let name = engine_name(status);
        let source = source(settings, status);
        let busy = self.engine_busy;
        let managed = status.map(|status| status.managed.clone());
        let installed = managed.as_ref().is_some_and(|managed| managed.installed);
        let can_download = status.is_some_and(|status| status.can_download);
        let downloaded = match &managed {
            Some(managed) if managed.installed => managed.version.as_ref().map_or_else(
                || "Installed".to_string(),
                |version| format!("Version {version}"),
            ),
            _ if status.is_some() && !can_download => {
                "No official download is available for this platform".into()
            }
            _ => "Official native build".into(),
        };
        let custom = if source == Source::Custom {
            settings.core.engine_path.clone()
        } else {
            "Stockfish executable".into()
        };
        let use_button = |id: &'static str, path: String| {
            Button::new(id)
                .outline()
                .small()
                .label("Use")
                .disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_engine_path(path.clone(), false, window, cx)
                }))
                .into_any_element()
        };
        let mut bundled_buttons = Vec::new();
        if source != Source::Bundled {
            bundled_buttons.push(use_button("engine-use-bundled", String::new()));
        }
        let mut downloaded_buttons = Vec::new();
        if installed && source != Source::Downloaded {
            let path = managed
                .as_ref()
                .map(|managed| managed.path.clone())
                .unwrap_or_default();
            downloaded_buttons.push(use_button("engine-use-downloaded", path));
        }
        if can_download {
            downloaded_buttons.push(
                Button::new("engine-download")
                    .outline()
                    .small()
                    .icon(icon("download"))
                    .label(if installed {
                        "Check for update"
                    } else {
                        "Download"
                    })
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.install_engine(window, cx)))
                    .into_any_element(),
            );
        }
        if installed {
            downloaded_buttons.push(
                Button::new("engine-delete")
                    .ghost()
                    .small()
                    .icon(icon("trash-2"))
                    .label("Delete")
                    .text_color(palette.error)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.ask_delete_engine(window, cx)))
                    .into_any_element(),
            );
        }
        let custom_buttons = vec![
            Button::new("engine-choose")
                .outline()
                .small()
                .icon(icon("folder-open"))
                .label("Choose file…")
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.choose_engine(window, cx)))
                .into_any_element(),
        ];
        let status_badge = if ready {
            widgets::badge("Ready", Look::Success, palette)
                .child(icon("circle-check").size(px(14.)))
        } else {
            widgets::badge("Not found", Look::Warning, palette)
                .child(icon("triangle-alert").size(px(14.)))
        };
        let mut rows = Rows::new(self.narrow()).row(
            "Status",
            Some(if ready && !name.is_empty() {
                name.into()
            } else {
                "Choose an available engine below".into()
            }),
            h_flex().child(status_badge),
            palette,
        );
        rows.rows.push(self.engine_option(
            "Bundled Stockfish 19 lite",
            "Included".into(),
            source == Source::Bundled,
            bundled_buttons,
            palette,
        ));
        rows.rows.push(self.engine_option(
            "Downloaded Stockfish",
            downloaded,
            source == Source::Downloaded,
            downloaded_buttons,
            palette,
        ));
        rows.rows.push(self.engine_option(
            "Your own executable",
            custom,
            source == Source::Custom,
            custom_buttons,
            palette,
        ));
        let review = cx.global::<ReviewHandle>().0.read(cx).status.clone();
        let auto = settings.core.review_auto;
        let review_rows = Rows::new(self.narrow())
            .row(
                "Review my games automatically",
                Some("Runs in the background after syncing".into()),
                choice(
                    "review-auto",
                    &[
                        (ReviewAuto::Off, "Off", None),
                        (ReviewAuto::Recent, "Last 30 days", None),
                        (ReviewAuto::All, "All", None),
                    ],
                    auto,
                    |s, value| s.core.review_auto = value,
                    palette,
                ),
                palette,
            )
            .switch(
                "review-battery",
                "Also on battery power",
                Some("Uses more power; reviews otherwise wait for a charger"),
                settings.core.review_on_battery,
                auto != ReviewAuto::Off,
                |s, on| s.core.review_on_battery = on,
                palette,
            );
        // `#settings-review`: one card, its title and status above the rows.
        v_flex().gap(px(16.)).child(rows.card(palette)).child(
            review_rows.card_under(
                Some(
                    info(
                        "Game review",
                        Some(review_text(&review, auto).into()),
                        palette,
                    )
                    .into_any_element(),
                ),
                palette,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{engine_name, review_text};
    use kchess_contracts::types::{EngineManaged, EngineStatus, ReviewAuto};
    use serde_json::json;

    fn status(path: &str, bundled: bool) -> EngineStatus {
        EngineStatus {
            identity: None,
            ready: true,
            path: path.into(),
            bundled,
            managed: EngineManaged {
                installed: true,
                path: "/managed/stockfish".into(),
                version: Some("17.1".into()),
            },
            can_download: true,
        }
    }

    #[test]
    fn the_engine_in_use_is_named() {
        assert_eq!(engine_name(Some(&status("", true))), "Bundled Stockfish");
        assert_eq!(
            engine_name(Some(&status("/managed/stockfish", false))),
            "Downloaded Stockfish 17.1"
        );
        assert_eq!(engine_name(Some(&status("/mine", false))), "Your Stockfish");
        assert_eq!(engine_name(None), "");
    }

    #[test]
    fn the_review_queue_is_described() {
        assert_eq!(
            review_text(
                &json!({ "current": { "done": 3, "total": 40 }, "waiting": 2 }),
                ReviewAuto::All
            ),
            "Reviewing a game: 3 of 40 positions · 2 more waiting."
        );
        assert_eq!(
            review_text(&json!({ "paused": "battery" }), ReviewAuto::All),
            "Paused while on battery power"
        );
        assert_eq!(
            review_text(&json!({ "waiting": 0 }), ReviewAuto::Off),
            "Games are reviewed when you ask for it"
        );
        assert_eq!(
            review_text(&json!({ "waiting": 4 }), ReviewAuto::Recent),
            "4 games waiting"
        );
        assert_eq!(
            review_text(&json!({ "waiting": 0 }), ReviewAuto::Recent),
            "All caught up"
        );
    }
}
