//! Settings → Updates (`AppUpdateSettings.vue`): this version, whether a newer one is out, its
//! download and installation, and the update preferences.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::Settings;

use super::{Rows, SettingsPage};
use crate::assets::icon;
use crate::theme::Palette;
use crate::updates::{Phase, Updates, UpdatesHandle};
use crate::widgets::{self, KeyboardFocus as _, Look};

/// `RESTART_UPDATE_DESCRIPTION`.
pub const RESTART_DESCRIPTION: &str = "KChess will close and reopen. Finish any game or timed puzzle run first; an unfinished local game or run will be lost, and an online game's clock will keep running.";

/// Ask, then restart into the downloaded update (the Updates section and the top bar's button).
pub fn ask_restart(window: &mut Window, cx: &mut App) {
    let handle = cx.global::<UpdatesHandle>().0.clone();
    let version = handle.read(cx).version.clone().unwrap_or_default();
    window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
        let handle = handle.clone();
        dialog
            .title(format!("Restart and install KChess {version}?"))
            .description(RESTART_DESCRIPTION)
            .ok_text("Restart and install")
            .ok_variant(ButtonVariant::Primary)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, _, cx| {
                // Unsaved work reaches the profile first, as on any quit (`Updates::install`).
                handle.update(cx, |updates, cx| updates.install(cx));
                true
            })
    });
}

fn bytes(count: u64) -> String {
    widgets::format_bytes(count as f64)
}

/// `timeAgo` for a check: "just now", "5 minutes ago", "3 hours ago".
fn ago(millis: i64) -> String {
    let minutes = (chrono::Utc::now().timestamp_millis() - millis).max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        1 => "1 minute ago".into(),
        2..60 => format!("{minutes} minutes ago"),
        60..120 => "1 hour ago".into(),
        _ => format!("{} hours ago", minutes / 60),
    }
}

/// The badge beside the version (`badge`).
fn badge(phase: Phase) -> Option<(&'static str, Look, &'static str)> {
    Some(match phase {
        Phase::UpToDate => ("Up to date", Look::Success, "circle-check"),
        Phase::Available => ("Update available", Look::Primary, "sparkles"),
        Phase::Downloading => ("Downloading", Look::Primary, "download"),
        Phase::Downloaded => ("Ready to install", Look::Primary, "rotate-cw"),
        Phase::Installing => ("Installing", Look::Primary, "rotate-cw"),
        Phase::Error => ("Update failed", Look::Error, "circle-alert"),
        Phase::Disabled => ("Manual updates", Look::NeutralSoft, "hand"),
        Phase::Idle | Phase::Checking => return None,
    })
}

/// One line under the version: what is happening, or what can be done next (`summary`).
fn summary(updates: &Updates, settings: &Settings) -> String {
    match updates.phase {
        Phase::Checking => "Checking for updates…".into(),
        Phase::Downloading => match updates.progress {
            Some((received, total)) if total > 0 => format!(
                "{}% · {} of {}",
                (100 * received / total).min(100),
                bytes(received),
                bytes(total)
            ),
            _ => "Preparing download…".into(),
        },
        Phase::Downloaded if settings.frontend.update_install_on_quit => {
            "Installs when you quit KChess, or restart now.".into()
        }
        Phase::Downloaded => "Restart KChess to install.".into(),
        Phase::Installing => "Installing the update…".into(),
        Phase::Disabled => {
            "This installation cannot update itself. Download and install the latest release."
                .into()
        }
        Phase::Idle => "Not checked yet".into(),
        _ => String::new(),
    }
}

impl SettingsPage {
    pub(super) fn updates(
        &self,
        settings: &Settings,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let handle = cx.global::<UpdatesHandle>().0.clone();
        let updates = handle.read(cx);
        let phase = updates.phase;
        let target = updates.version.clone().filter(|_| {
            matches!(
                phase,
                Phase::Available
                    | Phase::Downloading
                    | Phase::Downloaded
                    | Phase::Installing
                    | Phase::Error
            )
        });
        let mut line = summary(updates, settings);
        if let Some(checked) = updates.checked_at.filter(|_| target.is_none()) {
            if !line.is_empty() {
                line.push_str(" · ");
            }
            line.push_str(&format!("Checked {}", ago(checked)));
        }
        let page = updates.release_page();
        let act = |run: fn(&mut Updates, &mut Context<Updates>)| {
            let handle = handle.downgrade();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = handle.update(cx, run);
            }
        };
        let primary = match phase {
            Phase::Available => Some(
                Button::new("update-download")
                    .primary()
                    .icon(icon("download"))
                    .label("Download update")
                    .on_click(act(|updates, cx| updates.download(cx))),
            ),
            Phase::Downloaded => Some(
                Button::new("update-install")
                    .primary()
                    .icon(icon("rotate-cw"))
                    .label("Restart and install")
                    .on_click(|_, window, cx| ask_restart(window, cx)),
            ),
            Phase::Error if target.is_some() => Some(
                Button::new("update-retry")
                    .primary()
                    .icon(icon("rotate-ccw"))
                    .label("Retry download")
                    .on_click(act(|updates, cx| updates.download(cx))),
            ),
            Phase::Disabled => Some({
                let page = page.clone();
                Button::new("update-releases")
                    .primary()
                    .icon(icon("external-link"))
                    .label("Get latest release")
                    .on_click(move |_, _, cx| cx.open_url(&page))
            }),
            _ => None,
        };
        let check = (updates.can_check()
            && !matches!(
                phase,
                Phase::Downloaded | Phase::Downloading | Phase::Installing
            ))
        .then(|| {
            Button::new("update-check")
                .when(primary.is_some(), |button| button.ghost())
                .when(primary.is_none(), |button| button.outline())
                .icon(icon("refresh-cw"))
                .label("Check for updates")
                .loading(phase == Phase::Checking)
                .disabled(phase == Phase::Checking)
                .on_click(act(|updates, cx| updates.check(false, cx)))
        });
        let label = if target.is_some() {
            "What's new"
        } else {
            "Release notes"
        };
        let notes = || {
            let page = page.clone();
            div()
                .id("update-notes")
                .role(Role::Link)
                .keyboard_focus(palette.primary)
                .aria_label(label)
                .flex()
                .items_center()
                .gap(px(2.))
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .cursor_pointer()
                .hover(|link| link.text_color(palette.text))
                .on_click(move |_, _, cx| cx.open_url(&page))
                .child(label)
                .child(icon("arrow-up-right").size(px(12.)))
        };
        let head = h_flex()
            .gap(px(16.))
            .flex_wrap()
            .child(img("icon.png").size(px(52.)).flex_none())
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(200.))
                    .gap(px(4.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(palette.highlighted)
                                    .child(format!(
                                        "KChess {}",
                                        crate::updates::client_version(&updates.current)
                                    )),
                            )
                            .children(badge(phase).map(|(label, look, glyph)| {
                                widgets::icon_badge(icon(glyph), label, look, palette)
                            })),
                    )
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .flex_wrap()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .when(!line.is_empty(), |row| row.child(line.clone()))
                            .when(target.is_none() && phase != Phase::Disabled, |row| {
                                row.child(notes())
                            }),
                    ),
            )
            .child(h_flex().gap(px(8.)).children(primary).children(check));
        let percent = updates
            .progress
            .filter(|(_, total)| *total > 0)
            .map(|(received, total)| (received as f32 / total as f32).min(1.));
        let target_block = target.map(|version| {
            v_flex()
                .gap(px(8.))
                .pt(px(14.))
                .border_t_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .child(widgets::badge(
                            updates.current.clone(),
                            Look::NeutralSoft,
                            palette,
                        ))
                        .child(icon("arrow-right").size(px(14.)))
                        .child(widgets::badge(version, Look::Primary, palette)),
                )
                .child(
                    h_flex()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .children(updates.released.as_ref().and_then(|date| {
                            chrono::DateTime::parse_from_rfc3339(date)
                                .ok()
                                .map(|date| format!("Released {}", date.format("%b %-d, %Y")))
                        }))
                        .child(notes()),
                )
                .when(phase == Phase::Downloading, |block| {
                    block.child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded_full()
                            .bg(palette.accented)
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(palette.primary)
                                    .w(relative(percent.unwrap_or(0.05))),
                            ),
                    )
                })
        });
        let status = widgets::card(palette)
            .gap(px(14.))
            .child(head)
            .children(target_block)
            .children(updates.error.clone().map(|error| {
                h_flex()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(icon("circle-alert").size(px(14.)))
                    .child(error)
            }));
        let can_check = updates.can_check();
        let frontend = &settings.frontend;
        v_flex().gap(px(16.)).child(status).when(can_check, |page| {
            page.child(
                Rows::new(self.narrow())
                    .switch(
                        "update-auto-check",
                        "Automatically check for updates",
                        Some("On startup and every six hours."),
                        frontend.update_auto_check,
                        true,
                        |s, on| s.frontend.update_auto_check = on,
                        palette,
                    )
                    .switch(
                        "update-auto-download",
                        "Download updates in the background",
                        Some("Turning this off does not cancel an ongoing download."),
                        frontend.update_auto_download,
                        true,
                        |s, on| s.frontend.update_auto_download = on,
                        palette,
                    )
                    .switch(
                        "update-install-on-quit",
                        "Install updates when I quit",
                        Some("KChess will not quit or restart automatically."),
                        frontend.update_install_on_quit,
                        true,
                        |s, on| s.frontend.update_install_on_quit = on,
                        palette,
                    )
                    .card(palette),
            )
        })
    }
}
