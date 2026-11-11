//! The engine on the analysis board: Stockfish's search and Lichess's cloud evaluation for
//! the position shown, their lines and the engine panel.

use super::*;
use kchess_contracts::calls;
use kchess_contracts::check::{Bounded, Position};

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
            let status = status.await.unwrap_or(Value::Null);
            let _ = this.update(cx, |this, cx| {
                this.engine_ready = status["ready"].as_bool() == Some(true);
                let identity = status["identity"].as_str().map(str::to_string);
                if identity != this.engine_identity {
                    // Another engine: what the old one said is no longer this one's.
                    this.engine_identity = identity;
                    this.clear_evaluations();
                }
                this.engine_changed(cx);
            });
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
        self.cloud_request += 1;
        self.wanted.clear();
        self.engine_busy = false;
        stop_analysis(true, cx);
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
        self.client_id += 1;
        let request = self.client_id;
        self.engine_error = None;
        let engine_on = Self::engine_on(cx);
        let lines = prefs_lines(cx);
        let infinite = crate::prefs::get(cx, INFINITE, false);
        let position = self.position();
        if !engine_on
            || !self.engine_ready
            || position.is_none()
            || game_over(position.as_ref()).is_some()
            || !Self::assistance_allowed(cx)
            || self.reviewing(cx).is_some()
        {
            self.wanted.clear();
            self.engine_busy = false;
            // Turning analysis off releases the engine; other pauses keep it warm.
            stop_analysis(!engine_on, cx);
            cx.notify();
            return;
        }
        let context = self.engine_context();
        let known = self.evaluations.get(&context);
        if !infinite
            && known.is_some_and(|known| {
                known["reason"] == "completed"
                    && known["engine"].as_str() == self.engine_identity.as_deref()
                    && known["lines"].as_array().map_or(0, Vec::len) >= lines
            })
        {
            self.wanted = fen;
            self.engine_busy = false;
            stop_analysis(false, cx);
            cx.notify();
            return;
        }
        self.wanted = fen.clone();
        self.live = None;
        self.engine_busy = true;
        let mut asked = json!({
            "fen": fen,
            "clientId": request,
            "rootFen": if threat { fen.clone() } else { self.root.fen.clone() },
            "moves": if threat { Vec::new() } else { moves_of(&self.path) },
            "lines": lines.clamp(1, 5),
            "infinite": infinite,
        });
        // Fairy-Stockfish analyses the other variants.
        if let Some(variant) = self.fairy_variant() {
            asked["variant"] = json!(variant);
        }
        let start = cx
            .global::<AppCore>()
            .request(calls::StartAnalysis { request: asked });
        cx.spawn(async move |this, cx| {
            let started = start.await;
            let _ = this.update(cx, |this, cx| {
                let current = this.wanted == fen && this.client_id == request;
                match started {
                    Ok(id) if current => {
                        this.latest_id = this.latest_id.max(id.as_f64().unwrap_or_default());
                    }
                    Err(cause) if current => {
                        crate::log::write(
                            Level::Warn,
                            "analysis",
                            &format!("starting analysis failed: {}", cause.message),
                        );
                        this.engine_busy = false;
                        this.engine_error = Some(cause.message);
                        cx.notify();
                    }
                    _ => {}
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// An engine update: kept when it is for the search running now.
    pub(super) fn receive(&mut self, update: Value, cx: &mut Context<Self>) {
        let id = update["id"].as_f64().unwrap_or_default();
        if update["fen"].as_str() != Some(self.wanted.as_str())
            || update["clientId"].as_f64() != Some(self.client_id as f64)
            || update["context"].as_str() != Some(self.engine_context().as_str())
            || id < self.latest_id
        {
            return;
        }
        self.latest_id = id;
        self.remember(&update);
        if let Some(error) = update["error"].as_str() {
            self.engine_error = Some(error.to_string());
            self.engine_busy = false;
        } else if update["done"] == json!(true) {
            self.engine_busy = false;
        }
        self.live = Some(update);
        self.redraw(cx);
    }

    /// `remember`: keep the deepest evaluation of each position, a completed one over an
    /// interrupted one, and never a failed search.
    pub(super) fn remember(&mut self, update: &Value) {
        let key = update["context"]
            .as_str()
            .or(update["fen"].as_str())
            .unwrap_or_default()
            .to_string();
        let lines = update["lines"].as_array().map_or(0, Vec::len);
        if lines == 0 || !update["error"].is_null() || update["reason"] == "failed" {
            return;
        }
        if let Some(known) = self.evaluations.get(&key) {
            if known["reason"] == "completed" && update["reason"] != "completed" {
                let (old, new) = (known["engine"].as_str(), update["engine"].as_str());
                if old.is_none() || new.is_none() || old == new {
                    return;
                }
            }
            if known["depth"].as_f64() > update["depth"].as_f64()
                && known["lines"].as_array().map_or(0, Vec::len) >= lines
            {
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
    pub(super) fn evaluation(&self, cx: &App) -> Option<&Value> {
        if !Self::assistance_allowed(cx) {
            return None;
        }
        let context = self.engine_context();
        if let Some(live) = &self.live
            && live["fen"].as_str() == Some(self.engine_fen().as_str())
            && live["context"].as_str() == Some(context.as_str())
        {
            return Some(live);
        }
        self.evaluations.get(&context)
    }

    /* ── Lichess cloud evaluation ─────────────────────────────────────── */

    pub(super) fn ask_cloud(&mut self, cx: &mut Context<Self>) {
        self.cloud_task = None;
        self.cloud_request += 1;
        let request = self.cloud_request;
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
            let _ = this.update(cx, |this, cx| {
                this.cloud_busy = true;
                cx.notify();
            });
            let answer = core
                .request(calls::CloudEval {
                    fen: Position(fen),
                    lines: Bounded(lines as u64),
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if request != this.cloud_request {
                    return;
                }
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
            });
        }));
    }

    /// Lichess's lines for the position shown, when it has some.
    pub(super) fn cloud_lines(&self, cx: &App) -> Vec<Value> {
        match &self.cloud {
            Some(cloud) if Self::assistance_allowed(cx) && cloud.fen == self.node().fen => cloud
                .lines
                .iter()
                .filter_map(|line| serde_json::to_value(line).ok())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Without the local engine, the cloud's lines drive the bar, arrows and line list.
    pub(super) fn using_cloud(&self, cx: &App) -> bool {
        (!Self::engine_on(cx) || !self.engine_ready)
            && self.threat_position().is_none()
            && !self.cloud_lines(cx).is_empty()
    }

    /// The evaluation bar's line: the position's own, also while its threat is shown.
    pub(super) fn bar_line(&self, cx: &App) -> Option<Value> {
        if self.threat_position().is_some() {
            return self.evaluations.get(&self.context())?["lines"]
                .as_array()?
                .first()
                .cloned();
        }
        self.lines(cx).first().cloned()
    }

    pub(super) fn lines(&self, cx: &App) -> Vec<Value> {
        if self.using_cloud(cx) {
            self.cloud_lines(cx)
        } else {
            self.evaluation(cx)
                .and_then(|update| update["lines"].as_array().cloned())
                .unwrap_or_default()
        }
    }

    /// Play an engine line up to (and including) the move clicked.
    pub(super) fn play_line(&mut self, pv: &[String], up_to: usize, cx: &mut Context<Self>) {
        for uci in pv.iter().take(up_to + 1) {
            if !self.play(uci, cx) {
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
            .and_then(|line| line["pv"][0].as_str().map(str::to_string));
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
        if !self.engine_ready {
            return "Stockfish not found".into();
        }
        if let Some(error) = &self.engine_error {
            return error.clone();
        }
        if !Self::engine_on(cx) {
            return "Engine off".into();
        }
        if let Some(over) = game_over(self.position().as_ref()) {
            return over.into();
        }
        if !Self::assistance_allowed(cx) {
            return if cx.global::<OnlineHandle>().0.read(cx).phase == crate::online::Phase::Checking
            {
                "Checking Lichess game status…".into()
            } else {
                "Analysis paused until Lichess game status is verified".into()
            };
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
            update["depth"].as_f64().unwrap_or_default(),
            if infinite { "" } else { "/26" }
        );
        let nps = update["nps"].as_f64().unwrap_or_default();
        let speed = if nps >= 1e6 {
            format!(" · {:.1}M nodes/s", nps / 1e6)
        } else if nps > 0. {
            format!(" · {}k nodes/s", (nps / 1000.).round())
        } else {
            String::new()
        };
        let tail = if update["reason"] == "interrupted" {
            " · Interrupted — retry or change position"
        } else if update["done"] == json!(true) {
            ""
        } else {
            "…"
        };
        format!("{threat}{depth}{speed}{tail}")
    }

    /// The engine's head row, the cloud's line and the engine lines (`.ceval`).
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
                let described = |title: &'static str, hint: Option<&'static str>| {
                    v_flex()
                        .child(div().text_size(px(14.)).child(title))
                        .children(hint.map(|hint| {
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(hint)
                        }))
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
                                    .on_click(move |on, _, cx| {
                                        crate::prefs::set(cx, INFINITE, *on);
                                        let _ = on_infinite
                                            .update(cx, |page, cx| page.engine_changed(cx));
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
                                    .on_click(move |on, _, cx| {
                                        crate::prefs::set(cx, ARROWS, *on);
                                        let _ = on_arrows.update(cx, |page, cx| page.redraw(cx));
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
                    .disabled(!self.engine_ready || !allowed)
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
                            .truncate()
                            .child("Stockfish"),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .min_w_0()
                            .text_size(px(11.))
                            .text_color(if self.engine_error.is_some() {
                                palette.error
                            } else {
                                palette.text_muted
                            })
                            .when(engine_on && self.engine_busy, |row| {
                                row.child(crate::widgets::spinner(px(12.)))
                            })
                            .child(
                                div()
                                    .truncate()
                                    .font_features(crate::widgets::tabular())
                                    .child(status),
                            ),
                    ),
            )
            .child(settings_button);
        let threat = self.threat_position().is_some();
        let cloud_line = (cloud_on && allowed && !using_cloud && !threat).then(|| {
            let cloud_lines = self.cloud_lines(cx);
            let text: AnyElement = match cloud_lines.first() {
                Some(best) => {
                    let pv: Vec<String> =
                        serde_json::from_value(best["pv"].clone()).unwrap_or_default();
                    let first = pv_san(&self.node().fen, &pv, 1, self.root.variant().key())
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
        let body: Option<AnyElement> = if !self.engine_ready && !using_cloud {
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
                            variant: self.root.variant().key(),
                            lines: &lines,
                            pending: if using_cloud { 0 } else { prefs_lines(cx) },
                            ended: over.map(|_| "Nothing to analyse"),
                        }, // A threat's line starts from a position that is not on the board.
                        std::rc::Rc::new(move |pv: &[String], at, _: &mut Window, cx: &mut App| {
                            if !threat {
                                let _ = page.update(cx, |page, cx| page.play_line(pv, at, cx));
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

/// The engine lines chosen (1–5, three by default).
pub(super) fn prefs_lines(cx: &App) -> usize {
    crate::prefs::get::<usize>(cx, ENGINE_LINES, 3).clamp(1, 5)
}

/// `stopAnalysisSafe`: stop the search; `release` also closes the engine.
pub(super) fn stop_analysis(release: bool, cx: &mut App) {
    let stop = cx.global::<AppCore>().request(calls::StopAnalysis {
        kill: release.then(|| json!(true)),
    });
    cx.background_executor()
        .spawn(async move {
            if let Err(cause) = stop.await {
                crate::log::write(
                    Level::Warn,
                    "analysis",
                    &format!("stopping analysis failed: {}", cause.message),
                );
            }
        })
        .detach();
}
