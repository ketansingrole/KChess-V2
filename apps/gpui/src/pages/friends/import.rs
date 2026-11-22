//! The Friends page's import: the players the connected accounts follow on Lichess, to add.

use super::*;

/// The players the connected accounts follow on Lichess, to add.
pub struct ImportDialog {
    report: Option<FollowingReport>,
    loading: Loading,
    selected: HashSet<String>,
    show_removed: bool,
    filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ImportDialog {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> ImportDialog {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name"));
        let subscriptions = vec![cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        let mut dialog = ImportDialog {
            report: None,
            loading: Loading::default(),
            selected: HashSet::new(),
            show_removed: false,
            filter,
            _subscriptions: subscriptions,
        };
        dialog.load(cx);
        dialog
    }

    /// Ask Lichess afresh each time.
    fn load(&mut self, cx: &mut Context<Self>) {
        let following = cx.global::<AppCore>().request(calls::Following {});
        self.loading.load(
            cx,
            "friends",
            |this: &mut Self| &mut this.loading,
            following,
            |this, answer, _| {
                if let Ok(report) = answer {
                    this.report = Some(report);
                }
            },
        );
        cx.notify();
    }

    fn candidates(&self, cx: &App) -> Vec<FollowedUser> {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        self.report
            .iter()
            .flat_map(|report| &report.users)
            .filter(|user| !user.already_added)
            .filter(|user| self.show_removed || !user.dismissed)
            .filter(|user| needle.is_empty() || user.username.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let users: Vec<FollowedUser> = self
            .report
            .as_ref()
            .map(|report| report.users.clone())
            .unwrap_or_default();
        let candidates = self.candidates(cx);
        let already = users.iter().filter(|user| user.already_added).count();
        let removed = users
            .iter()
            .filter(|user| user.dismissed && !user.already_added)
            .count();
        let all_selected = !candidates.is_empty()
            && candidates
                .iter()
                .all(|user| self.selected.contains(&user.username));
        let several = crate::accounts::handle(cx).read(cx).connected().len() > 1;
        let problems: Vec<FollowingProblem> = self
            .report
            .as_ref()
            .map(|report| report.problems.clone())
            .unwrap_or_default();
        let count = self.selected.len();
        let body: AnyElement =
            if self.loading.busy() {
                h_flex()
                    .gap(px(8.))
                    .text_color(palette.text_muted)
                    .child(crate::widgets::spinner(px(14.)))
                    .child("Asking Lichess…")
                    .into_any_element()
            } else if self.report.is_some() {
                v_flex()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(div().flex_1().child(
                                Input::new(&self.filter).aria_label("Filter followed players"),
                            ))
                            .child(
                                Button::new("import-all")
                                    .outline()
                                    .label(if all_selected { "Clear" } else { "Select all" })
                                    .disabled(candidates.is_empty())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let names: Vec<String> = this
                                            .candidates(cx)
                                            .into_iter()
                                            .map(|user| user.username)
                                            .collect();
                                        if all_selected {
                                            this.selected.clear();
                                        } else {
                                            this.selected.extend(names);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(if candidates.is_empty() {
                        widgets::empty_state(
                            icon("users"),
                            if users.is_empty() {
                                "No followed players found"
                            } else {
                                "Nobody new to add"
                            },
                            Some(
                                if users.is_empty() {
                                    "Follow players on lichess.org and they will show up here"
                                } else {
                                    "Everyone they follow is already in Following"
                                }
                                .into(),
                            ),
                            &palette,
                        )
                        .into_any_element()
                    } else {
                        v_flex()
                            .id("import-list")
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .children(candidates.iter().enumerate().map(|(index, user)| {
                                let name = user.username.clone();
                                let checked = self.selected.contains(&name);
                                let mut ratings = Vec::new();
                                if let Some(blitz) = user.ratings.blitz {
                                    ratings.push(format!("Blitz {blitz}"));
                                }
                                if let Some(rapid) = user.ratings.rapid {
                                    ratings.push(format!("Rapid {rapid}"));
                                }
                                let via: Vec<String> = user
                                    .followed_by
                                    .iter()
                                    .map(|name| format!("@{name}"))
                                    .collect();
                                let toggled = name.clone();
                                h_flex()
                                    .gap(px(8.))
                                    .py(px(6.))
                                    .child(
                                        Checkbox::new(("import-user", index))
                                            .checked(checked)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.selected.remove(&toggled) {
                                                    this.selected.insert(toggled.clone());
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(format!("@{name}")),
                                    )
                                    .children(user.title.clone().map(|title| {
                                        widgets::badge(title, Look::Primary, &palette)
                                    }))
                                    .when(user.dismissed, |row| {
                                        row.child(widgets::badge(
                                            "Removed before",
                                            Look::NeutralSoft,
                                            &palette,
                                        ))
                                    })
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child(ratings.join(" · ")),
                                    )
                                    .when(several, |row| {
                                        row.child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(palette.text_muted)
                                                .child(format!("via {}", via.join(", "))),
                                        )
                                    })
                            }))
                            .into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("{already} already added"))
                            .when(removed > 0, |row| {
                                row.child(
                                    div()
                                        .id("import-removed")
                                        .role(Role::Button)
                                        .keyboard_focus(palette.primary)
                                        .aria_label(format!(
                                            "{} {removed} you removed earlier",
                                            if self.show_removed { "Hide" } else { "Show" }
                                        ))
                                        .underline()
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_removed = !this.show_removed;
                                            cx.notify();
                                        }))
                                        .child(format!(
                                            "{} {removed} you removed earlier",
                                            if self.show_removed { "Hide" } else { "Show" }
                                        )),
                                )
                            }),
                    )
                    .into_any_element()
            } else {
                div().into_any_element()
            };
        v_flex()
            .gap(px(12.))
            .child(widgets::hint(
                "Players your connected accounts follow on Lichess",
                &palette,
            ))
            .children(problems.iter().map(|problem| {
                h_flex()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(radii::BOX)
                    .bg(crate::theme::tint(palette.warning, 12.))
                    .text_size(px(13.))
                    .child(icon("triangle-alert").size(px(14.)))
                    .child(format!(
                        "@{}: couldn't read who it follows. {}",
                        problem.account, problem.message
                    ))
            }))
            .children(self.loading.error().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(format!("Couldn't reach Lichess: {error}"))
            }))
            .child(body)
            .child(
                h_flex()
                    .justify_between()
                    .pt(px(8.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("{count} selected")),
                    )
                    .child(
                        Button::new("import-add")
                            .primary()
                            .icon(icon("user-plus"))
                            .label(format!(
                                "Add {} {}",
                                if count > 0 {
                                    count.to_string()
                                } else {
                                    String::new()
                                },
                                if count == 1 { "player" } else { "players" }
                            ))
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let names: Vec<String> = this.selected.iter().cloned().collect();
                                crate::accounts::handle(cx)
                                    .update(cx, |accounts, cx| accounts.import(names, window, cx));
                                window.close_dialog(cx);
                            })),
                    ),
            )
    }
}
