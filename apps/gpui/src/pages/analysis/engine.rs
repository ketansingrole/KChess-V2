//! The engine on the analysis board: Stockfish's search and Lichess's cloud evaluation for
//! the position shown, their lines and the engine panel.

use super::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{Bounded, Position};
use kchess_contracts::types::{AnalysisReason, AnalysisRequest, EngineLine};

/// The spinner's place before the engine's status while it is on, and the gap after it: the
/// engine's name above starts as far in, so the two lines share a left edge.
const SPINNER_SLOT: Pixels = px(12.);
const SPINNER_GAP: Pixels = px(4.);

impl AnalysisPage {
    pub(crate) fn engine_on(cx: &App) -> bool {
        crate::prefs::get(cx, ENGINE_ON, true)
    }

    /// Practising a position: the engine (and anything else that knows) would tell.
    pub(super) fn hides_engine(&self) -> bool {
        self.practice
            .as_ref()
            .is_some_and(practice::Practice::hides_engine)
    }

    pub(super) fn assistance_allowed(cx: &App) -> bool {
        cx.global::<OnlineHandle>().0.read(cx).assistance_allowed()
    }

    /// Why engine help is held back, if it is.
    pub(super) fn pause(cx: &App) -> Option<crate::online::Pause> {
        cx.global::<OnlineHandle>().0.read(cx).pause()
    }

    /// The engine can analyse, as the core last said.
    pub(super) fn engine_ready(&self) -> bool {
        self.engine_ready == Some(true)
    }

    /// Start following the board with the engine (the page is shown).
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.attached {
            return;
        }
        self.attached = true;
        let status = cx
            .global::<AppCore>()
            .request(calls::EngineStatus { variant: None });
        cx.spawn(async move |this, cx| {
            let status = status.await.warn_on_err();
            this.update(cx, |this, cx| {
                this.engine_ready = Some(status.as_ref().is_some_and(|status| status.ready));
                let identity = status.and_then(|status| status.identity);
                if identity != this.engine_identity {
                    // Another engine: what the old one said is no longer this one's.
                    this.engine_identity = identity;
                    this.clear_evaluations();
                }
                this.engine_changed(cx);
            })
            .ok();
        })
        .detach();
        self.ask_cloud(cx);
        self.analyse(cx);
    }

    /// Stop the engine when the page is left.
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        self.attached = false;
        self.cloud_task = None;
        self.search.stop(true, cx);
    }

    pub(super) fn engine_changed(&mut self, cx: &mut Context<Self>) {
        self.analyse(cx);
        self.ask_cloud(cx);
        self.redraw(cx);
    }

    /// `analyse`: search the position shown, unless the engine is off, nothing is left to search,
    /// or a finished search with enough lines is already known.
    pub(super) fn analyse(&mut self, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        let fen = self.engine_fen();
        let threat = self.threat_position().is_some();
        let engine_on = Self::engine_on(cx);
        let lines = prefs_lines(cx);
        let infinite = crate::prefs::get(cx, INFINITE, false);
        let position = self.position();
        if !engine_on
            || !self.engine_ready()
            || position.is_none()
            || game_over(position.as_ref()).is_some()
            || !Self::assistance_allowed(cx)
            || self.reviewing(cx).is_some()
        {
            // Turning analysis off releases the engine; other pauses keep it warm.
            self.search.stop(!engine_on, cx);
            cx.notify();
            return;
        }
        let context = self.engine_context();
        let known = self.evaluations.get(&context);
        if !infinite
            && known.is_some_and(|known| {
                known.reason == Some(AnalysisReason::Completed)
                    && Some(known.engine.as_str()) == self.engine_identity.as_deref()
                    && known.lines.len() >= lines
            })
        {
            self.search.stop(false, cx);
            cx.notify();
            return;
        }
        let asked = AnalysisRequest {
            fen: fen.clone(),
            lines: lines.clamp(1, 5) as u32,
            infinite: Some(infinite),
            root_fen: Some(if threat {
                fen.clone()
            } else {
                self.root.fen.clone()
            }),
            moves: Some(if threat {
                Vec::new()
            } else {
                moves_of(&self.path)
            }),
            client_id: None,
            // Fairy-Stockfish analyses the other variants.
            variant: self
                .fairy_variant()
                .map(|variant| variant.as_str().to_string()),
        };
        // Every look searches again: the board asks only when what it shows changed, or to retry.
        self.search
            .restart(Some(asked), |page: &mut AnalysisPage| &mut page.search, cx);
        cx.notify();
    }

    /// The client id of the search the board last sent, and whether the engine is searching.
    #[cfg(test)]
    pub(crate) fn engine_search(&self) -> (Option<u64>, bool) {
        (self.search.started(), self.search.busy)
    }

    /// The position the board's search is for, and its context.
    #[cfg(test)]
    pub(crate) fn engine_target(&self) -> (String, String) {
        (self.engine_fen(), self.engine_context())
    }

    /// An engine update: kept when it is for the search running now.
    pub(super) fn receive(&mut self, update: AnalysisUpdate, cx: &mut Context<Self>) {
        if !self.search.receive(&update) {
            return;
        }
        self.remember(&update);
        self.redraw(cx);
    }

    /// `remember`: keep the deepest evaluation of each position, a completed one over an
    /// interrupted one, and never a failed search.
    pub(super) fn remember(&mut self, update: &AnalysisUpdate) {
        let key = update.context.clone();
        let lines = update.lines.len();
        if lines == 0 || update.error.is_some() || update.reason == Some(AnalysisReason::Failed) {
            return;
        }
        if let Some(known) = self.evaluations.get(&key) {
            let completed = Some(AnalysisReason::Completed);
            if known.reason == completed
                && update.reason != completed
                && known.engine == update.engine
            {
                return;
            }
            if known.depth > update.depth && known.lines.len() >= lines {
                return;
            }
        }
        self.order.retain(|entry| *entry != key);
        self.order.push_back(key.clone());
        self.evaluations.insert(key, update.clone());
        while self.order.len() > CACHE_SIZE {
            if let Some(oldest) = self.order.pop_front() {
                self.evaluations.remove(&oldest);
            }
        }
    }

    /// The deepest evaluation known for the position shown.
    pub(super) fn evaluation(&self, cx: &App) -> Option<&AnalysisUpdate> {
        if !Self::assistance_allowed(cx) {
            return None;
        }
        let context = self.engine_context();
        if let Some(live) = &self.search.update
            && live.fen == self.engine_fen()
            && live.context == context
        {
            return Some(live);
        }
        self.evaluations.get(&context)
    }

    /* ── Lichess cloud evaluation ─────────────────────────────────────── */

    pub(super) fn ask_cloud(&mut self, cx: &mut Context<Self>) {
        self.cloud_task = None;
        let fen = self.node().fen.clone();
        if self.cloud.as_ref().map(|cloud| cloud.fen.as_str()) != Some(fen.as_str()) {
            self.cloud = None;
        }
        self.cloud_error = None;
        let cloud_on = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .core
            .cloud_eval;
        if !self.attached
            || !cloud_on
            || !self.standard_rules()
            || !Self::assistance_allowed(cx)
            || game_over(self.position().as_ref()).is_some()
        {
            self.cloud = None;
            self.cloud_busy = false;
            return;
        }
        let lines = prefs_lines(cx).clamp(1, 5);
        let core = cx.global::<AppCore>().clone();
        self.cloud_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOUD_DELAY).await;
            this.update(cx, |this, cx| {
                this.cloud_busy = true;
                cx.notify();
            })
            .ok();
            let answer = core
                .lookup(calls::CloudEval {
                    fen: Position(fen),
                    lines: Bounded(lines as u64),
                })
                .await;
            this.update(cx, |this, cx| {
                this.cloud_busy = false;
                match answer {
                    Ok(cloud) => this.cloud = cloud,
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "analysis",
                            &format!("cloud evaluation failed: {}", cause.message),
                        );
                        this.cloud_error = Some(cause.message);
                    }
                }
                this.redraw(cx);
            })
            .ok();
        }));
    }

    /// Lichess's lines for the position shown, when it has some.
    pub(super) fn cloud_lines(&self, cx: &App) -> Vec<EngineLine> {
        match &self.cloud {
            Some(cloud) if Self::assistance_allowed(cx) && cloud.fen == self.node().fen => cloud
                .lines
                .iter()
                .map(|line| EngineLine {
                    rank: line.rank,
                    depth: line.depth as f64,
                    cp: line.cp.map(|cp| cp as f64),
                    mate: line.mate.map(|mate| mate as f64),
                    pv: line.pv.clone(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Without the local engine, the cloud's lines drive the bar, arrows and line list.
    pub(super) fn using_cloud(&self, cx: &App) -> bool {
        (!Self::engine_on(cx) || !self.engine_ready())
            && self.threat_position().is_none()
            && !self.cloud_lines(cx).is_empty()
    }

    /// The evaluation bar's line: the position's own, also while its threat is shown.
    pub(super) fn bar_line(&self, cx: &App) -> Option<EngineLine> {
        if self.threat_position().is_some() {
            return self
                .evaluations
                .get(&self.context())?
                .lines
                .first()
                .cloned();
        }
        self.lines(cx).first().cloned()
    }

    pub(super) fn lines(&self, cx: &App) -> Vec<EngineLine> {
        if self.using_cloud(cx) {
            self.cloud_lines(cx)
        } else {
            self.evaluation(cx)
                .map(|update| update.lines.clone())
                .unwrap_or_default()
        }
    }

    /// Play an engine line up to (and including) the move clicked.
    /// Only its last move sounds: the line is played at once, not a burst of clips.
    pub(super) fn play_line(&mut self, pv: &[String], up_to: usize, cx: &mut Context<Self>) {
        let line = &pv[..pv.len().min(up_to + 1)];
        for (index, uci) in line.iter().enumerate() {
            self.quiet_moves = index + 1 < line.len();
            let kept = self.play(uci, cx);
            self.quiet_moves = false;
            if !kept {
                break;
            }
        }
    }

    pub(super) fn play_best(&mut self, cx: &mut Context<Self>) {
        // The threat's best move is the opponent's, not one to play here.
        if self.hides_engine() || self.threat_position().is_some() {
            return;
        }
        let best = self
            .lines(cx)
            .first()
            .and_then(|line| line.pv.first().cloned());
        if let Some(uci) = best {
            self.play(&uci, cx);
        }
    }

    pub(super) fn engine_status(&self, cx: &App) -> String {
        if self.using_cloud(cx)
            && let Some(cloud) = &self.cloud
        {
            return format!(
                "Lichess cloud · depth {} · {}M nodes",
                cloud.depth,
                (cloud.knodes as f64 / 1000.).round()
            );
        }
        match self.engine_ready {
            None => return String::new(),
            Some(false) => return "Stockfish not found".into(),
            Some(true) => {}
        }
        if let Some(error) = &self.search.error {
            return error.clone();
        }
        if !Self::engine_on(cx) {
            return "Engine off".into();
        }
        if let Some(over) = game_over(self.position().as_ref()) {
            return over.into();
        }
        if let Some(pause) = Self::pause(cx) {
            return pause.text().into();
        }
        let threat = if self.threat_position().is_some() {
            "Threat · "
        } else {
            ""
        };
        let Some(update) = self.evaluation(cx) else {
            return format!("{threat}Starting…");
        };
        let infinite = crate::prefs::get(cx, INFINITE, false);
        let depth = format!(
            "Depth {}{}",
            update.depth,
            if infinite { "" } else { "/26" }
        );
        let nps = update.nps.unwrap_or_default();
        let speed = if nps >= 1e6 {
            format!(" · {:.1}M nodes/s", nps / 1e6)
        } else if nps > 0. {
            format!(" · {}k nodes/s", (nps / 1000.).round())
        } else {
            String::new()
        };
        let tail = if update.reason == Some(AnalysisReason::Interrupted) {
            " · Interrupted — retry or change position"
        } else if update.done {
            ""
        } else {
            "…"
        };
        format!("{threat}{depth}{speed}{tail}")
    }

    /// The engine's head row, the cloud's line and the engine lines.
    pub(super) fn render_engine(
        &self,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement {
        let engine_on = Self::engine_on(cx);
        let allowed = Self::assistance_allowed(cx);
        let using_cloud = self.using_cloud(cx);
        let lines = self.lines(cx);
        let position = self.position();
        let over = game_over(position.as_ref());
        let result = over.and(position.as_ref()).map(result_of);
        let score = if !engine_on && !using_cloud {
            "—".to_string()
        } else {
            result
                .map(str::to_string)
                .unwrap_or_else(|| format_eval(lines.first()))
        };
        let settings = cx.global::<SettingsHandle>().0.read(cx).settings.clone();
        let cloud_on = settings.core.cloud_eval;
        let lines_choice = self.lines_choice.clone();
        let page = cx.entity().downgrade();
        let settings_button = Popover::new("engine-settings")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("engine-settings-button")
                    .ghost()
                    .small()
                    .icon(icon("settings-2"))
                    .tooltip("Engine settings")
                    .accessibility_label("Engine settings"),
            )
            .content(move |_, _, cx| {
                let engine_on = AnalysisPage::engine_on(cx);
                let infinite = crate::prefs::get(cx, INFINITE, false);
                let arrows = crate::prefs::get(cx, ARROWS, true);
                let cloud_on = cx
                    .global::<SettingsHandle>()
                    .0
                    .read(cx)
                    .settings
                    .core
                    .cloud_eval;
                let allowed = AnalysisPage::assistance_allowed(cx);
                let palette = *cx.global::<Palette>();
                let (on_infinite, on_arrows, on_threat) =
                    (page.clone(), page.clone(), page.clone());
                let threat = page.upgrade().is_some_and(|page| page.read(cx).threat);
                // As in Settings: the title and hint take what the switch leaves, so a long hint
                // wraps inside the popover instead of running past its edge.
                let described = |title: &'static str, hint: Option<&'static str>| {
                    crate::widgets::setting_info(
                        title,
                        hint.map(SharedString::new_static),
                        &palette,
                    )
                    .flex_1()
                };
                v_flex()
                    .w(px(260.))
                    .gap(px(12.))
                    .p(px(4.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Switch::new("infinite")
                                    .large()
                                    .checked(infinite)
                                    .disabled(!engine_on)
                                    .accessibility_label("Infinite analysis")
                                    .on_click(move |on, window, cx| {
                                        let on = *on;
                                        let set = {
                                            let page = on_infinite.clone();
                                            move |on: bool, cx: &mut App| {
                                                crate::prefs::set(cx, INFINITE, on);
                                                let _ = page
                                                    .update(cx, |page, cx| page.engine_changed(cx));
                                            }
                                        };
                                        set(on, cx);
                                        crate::setting_notice::offer(
                                            crate::setting_notice::switched(
                                                "Infinite analysis",
                                                on,
                                            ),
                                            move |_, cx| set(!on, cx),
                                            window,
                                            cx,
                                        );
                                    }),
                            )
                            .child(described(
                                "Infinite analysis",
                                Some("Keep searching past depth 26"),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Switch::new("arrows")
                                    .large()
                                    .checked(arrows)
                                    .disabled(!engine_on)
                                    .accessibility_label("Show engine arrows")
                                    .on_click(move |on, window, cx| {
                                        let on = *on;
                                        let set = {
                                            let page = on_arrows.clone();
                                            move |on: bool, cx: &mut App| {
                                                crate::prefs::set(cx, ARROWS, on);
                                                page.update(cx, |page, cx| page.redraw(cx)).ok();
                                            }
                                        };
                                        set(on, cx);
                                        crate::setting_notice::offer(
                                            crate::setting_notice::switched("Engine arrows", on),
                                            move |_, cx| set(!on, cx),
                                            window,
                                            cx,
                                        );
                                    }),
                            )
                            .child(described("Show engine arrows", None)),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Switch::new("threat")
                                    .large()
                                    .checked(threat)
                                    .disabled(!engine_on)
                                    .accessibility_label("Show threat")
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            on_threat.update(cx, |page, cx| page.toggle_threat(cx));
                                    }),
                            )
                            .child(described(
                                "Show threat",
                                Some("The opponent's best move if it were their turn (X)"),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Switch::new("cloud")
                                    .large()
                                    .checked(cloud_on)
                                    .disabled(!allowed)
                                    .accessibility_label("Lichess cloud evaluation")
                                    .on_click(|on, _, cx| {
                                        let on = *on;
                                        crate::settings::change(cx, move |s| {
                                            s.core.cloud_eval = on
                                        });
                                    }),
                            )
                            .child(described(
                                "Lichess cloud evaluation",
                                Some("Sends positions to Lichess"),
                            )),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_size(px(14.)).child("Engine lines"))
                            .child(
                                div().w(px(112.)).child(
                                    Select::new(&lines_choice)
                                        .small()
                                        .disabled(!engine_on && !cloud_on)
                                        .accessibility_label("Engine lines"),
                                ),
                            ),
                    )
            });
        let status = self.engine_status(cx);
        let head = h_flex()
            .gap(px(8.))
            .child(
                Switch::new("engine-on")
                    .large()
                    .checked(engine_on)
                    .disabled(!self.engine_ready() || !allowed)
                    .accessibility_label("Engine analysis")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_engine(cx))),
            )
            .child(
                div()
                    .min_w(px(75.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if !engine_on && !using_cloud {
                        palette.dimmed
                    } else {
                        palette.text
                    })
                    .child(score),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            // In line with the status under it, past the spinner's place.
                            .when(engine_on, |name| name.pl(SPINNER_SLOT + SPINNER_GAP))
                            .truncate()
                            .child("Stockfish"),
                    )
                    .child(
                        h_flex()
                            .gap(SPINNER_GAP)
                            .min_w_0()
                            .text_size(px(11.))
                            .text_color(if self.search.error.is_some() {
                                palette.error
                            } else {
                                palette.text_muted
                            })
                            // The spinner's place is kept while the engine is on, so the
                            // depth beside it holds still as each search starts and ends.
                            .when(engine_on, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .size(SPINNER_SLOT)
                                        .when(self.search.busy, |slot| {
                                            slot.child(crate::widgets::spinner(SPINNER_SLOT))
                                        }),
                                )
                            })
                            .child(
                                div()
                                    .id("engine-status")
                                    .test_support()
                                    .truncate()
                                    .font_features(crate::widgets::tabular())
                                    .child(status),
                            )
                            .when(
                                Self::pause(cx) == Some(crate::online::Pause::Unverified),
                                |row| {
                                    row.child(crate::online::check_again_button(
                                        "engine-check-again",
                                    ))
                                },
                            ),
                    ),
            )
            .child(settings_button);
        let threat = self.threat_position().is_some();
        let cloud_line = (cloud_on && allowed && !using_cloud && !threat).then(|| {
            let cloud_lines = self.cloud_lines(cx);
            let text: AnyElement = match cloud_lines.first() {
                Some(best) => {
                    let pv = best.pv.clone();
                    let first = pv_san(&self.node().fen, &pv, 1, self.root.variant())
                        .first()
                        .map(|played| played.san.clone())
                        .unwrap_or_default();
                    h_flex()
                        .gap(px(4.))
                        .child(format!(
                            "Lichess cloud: {} at depth {} ·",
                            format_eval(Some(best)),
                            self.cloud
                                .as_ref()
                                .map(|cloud| cloud.depth)
                                .unwrap_or_default()
                        ))
                        .child(
                            div()
                                .id("cloud-play")
                                .role(Role::Button)
                                .keyboard_focus(palette.primary)
                                .aria_label(format!("Play {first}"))
                                .underline()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.play_line(&pv, pv.len().saturating_sub(1), cx)
                                }))
                                .child(format!("play {first}")),
                        )
                        .into_any_element()
                }
                None => div()
                    .child(if self.cloud_busy {
                        "Asking Lichess cloud…".to_string()
                    } else {
                        self.cloud_error.clone().unwrap_or_else(|| {
                            "Lichess cloud has no evaluation of this position".into()
                        })
                    })
                    .into_any_element(),
            };
            h_flex()
                .gap(px(4.))
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(icon("cloud").size(px(12.)))
                .child(text)
        });
        let body: Option<AnyElement> = if self.engine_ready == Some(false) && !using_cloud {
            Some(
                h_flex()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(
                        div()
                            .id("setup-stockfish")
                            .role(Role::Link)
                            .keyboard_focus(palette.primary)
                            .aria_label("Set up Stockfish")
                            .underline()
                            .cursor_pointer()
                            .on_click(|_, window, cx| {
                                crate::shell::open_settings(
                                    crate::pages::settings::Section::Engine,
                                    window,
                                    cx,
                                )
                            })
                            .child("Set up Stockfish"),
                    )
                    .child("to see evaluations.")
                    .into_any_element(),
            )
        } else if allowed && (engine_on || using_cloud) {
            Some(
                {
                    let page = cx.entity().downgrade();
                    crate::engine_view::engine_lines(
                        crate::engine_view::EngineLines {
                            fen: &self.engine_fen(),
                            variant: self.root.variant(),
                            orientation: self.orientation,
                            lines: &lines,
                            pending: if using_cloud { 0 } else { prefs_lines(cx) },
                            ended: over.map(|_| "Nothing to analyse"),
                        }, // A threat's line starts from a position that is not on the board.
                        std::rc::Rc::new(move |pv: &[String], at, _: &mut Window, cx: &mut App| {
                            if !threat {
                                page.update(cx, |page, cx| page.play_line(pv, at, cx)).ok();
                            }
                        }),
                        palette,
                        crate::notation::figurine(cx),
                    )
                }
                .into_any_element(),
            )
        } else {
            None
        };
        v_flex()
            .flex_none()
            .gap(px(8.))
            .child(head)
            .children(cloud_line)
            .children(body)
    }
}

impl AnalysisPage {
    /// Show `count` engine lines (the Undo of a change in the engine settings).
    pub(super) fn set_lines(&mut self, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, ENGINE_LINES, count);
        self.lines_choice.update(cx, |choice, cx| {
            choice.set_selected_index(Some(IndexPath::new(count - 1)), window, cx);
        });
        self.engine_changed(cx);
    }
}

/// The engine lines chosen (1–5, three by default).
pub(super) fn prefs_lines(cx: &App) -> usize {
    crate::prefs::get::<usize>(cx, ENGINE_LINES, 3).clamp(1, 5)
}
