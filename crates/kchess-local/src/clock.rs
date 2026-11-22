//! The Over the board page's chess clock: its two faces, presses, pauses and their history.

use super::*;

impl LocalPage {
    pub(super) fn clock_snapshot(&self) -> Snapshot {
        Snapshot {
            source: HistoryKind::Clock,
            setup: Setup::standard(),
            moves: Vec::new(),
            white: "Bottom player".into(),
            black: "Top player".into(),
            result: "*".into(),
            reason: self.clock.reason(),
            time_control: self.clock.time_control(),
            clock_summary: Some(self.clock.summary()),
        }
    }

    fn clock_changed(&mut self, cx: &mut Context<Self>) {
        if self.clock.running.is_some() && self.clock_ticker.is_none() {
            self.clock_ticker = Some(crate::timer::every(crate::timer::TICK, cx, |this, cx| {
                let now = this.now();
                if this.clock.expire(now) {
                    crate::sound::play(crate::sound::Sound::LowTime, cx);
                    this.clock_flagged(cx);
                }
                let shown =
                    [Side::Top, Side::Bottom].map(|side| clock_face(this.clock.left(side, now)));
                if this.clock_faces.changed(shown) {
                    cx.notify();
                }
                true
            }));
        } else if self.clock.running.is_none() {
            self.clock_ticker = None;
        }
        cx.notify();
    }

    /// A flag fell: the session is saved to history as it ended.
    fn clock_flagged(&mut self, cx: &mut Context<Self>) {
        let (source, has_play) = (self.clock_snapshot(), self.clock.has_play());
        if let Some(history) = &mut self.clock_history {
            history.save(true, source, has_play, cx);
        }
        self.clock_changed(cx);
    }

    fn press(&mut self, side: Side, cx: &mut Context<Self>) {
        if self.clock.press(side, self.now()) {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
            self.clock_flagged(cx);
        }
        self.clock_changed(cx);
    }

    fn start_or_pause(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        let fell = if self.clock.running.is_some() {
            self.clock.pause(now)
        } else {
            if self.clock.flagged.is_some() {
                self.reset_clock_history(cx);
            }
            self.clock.start(now)
        };
        if fell {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
            self.clock_flagged(cx);
        }
        self.clock_changed(cx);
    }

    fn reset_clock(&mut self, cx: &mut Context<Self>) {
        self.reset_clock_history(cx);
        self.clock.reset();
        self.clock_changed(cx);
    }

    fn reset_clock_history(&mut self, cx: &mut Context<Self>) {
        let (source, has_play) = (self.clock_snapshot(), self.clock.has_play());
        if let Some(history) = &mut self.clock_history {
            history.reset(source, has_play, cx);
        }
    }

    pub(super) fn configure_clock(&mut self, cx: &mut Context<Self>) {
        let read = |input: &Entity<InputState>, cx: &App| {
            input.read(cx).value().trim().parse::<f64>().ok()
        };
        let [top, bottom, increment] = &self.clock_inputs;
        let (top, bottom, increment) = (read(top, cx), read(bottom, cx), read(increment, cx));
        let config = Config {
            minutes: top
                .filter(|m| *m >= 1.)
                .unwrap_or(self.clock.config.minutes),
            bottom_minutes: bottom
                .filter(|m| *m >= 1.)
                .unwrap_or(self.clock.config.bottom_minutes),
            increment: increment
                .filter(|s| *s >= 0.)
                .unwrap_or(self.clock.config.increment),
        };
        self.clock.configure(config);
        cx.notify();
    }

    /// The chess clock's keys (`CLOCK_KEYS`) when the page itself has focus (not a text field).
    pub(super) fn clock_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::board::page_has_keys(&self.focus, window, cx)
            || event.keystroke.modifiers.modified()
        {
            return;
        }
        let Some(action) = page_key(CLOCK_KEYS, &event.keystroke.key) else {
            return;
        };
        match action {
            ClockKey::Press => {
                let side = self.clock.running.unwrap_or(Side::Top);
                self.press(side, cx);
            }
            ClockKey::Pause => {
                if self.clock.pause(self.now()) {
                    self.clock_flagged(cx);
                }
                self.clock_changed(cx);
            }
        }
        cx.stop_propagation();
    }

    /// How far a clock face is turned (`kchess:clock-rotation`), so a player across the table
    /// reads it the right way up.
    fn rotation(side: Side, cx: &App) -> u16 {
        let turned: serde_json::Value =
            crate::prefs::get(cx, ROTATION_KEY, json!({ "top": 0, "bottom": 0 }));
        let key = if side == Side::Top { "top" } else { "bottom" };
        (turned[key].as_f64().unwrap_or_default() as u16 / 90 * 90) % 360
    }

    /// A quarter turn clockwise.
    fn rotate(side: Side, cx: &mut App) {
        let mut turned: serde_json::Value =
            crate::prefs::get(cx, ROTATION_KEY, json!({ "top": 0, "bottom": 0 }));
        let key = if side == Side::Top { "top" } else { "bottom" };
        turned[key] = json!((Self::rotation(side, cx) + 90) % 360);
        crate::prefs::set(cx, ROTATION_KEY, turned);
    }

    pub(super) fn render_clock(
        &mut self,
        palette: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = self.now();
        let vertical = clock_layout(cx) == ClockLayout::Vertical;
        // The clocks take the board's place, so switching modes moves nothing else: its side
        // across, and its column's height beside the panel (its side again when stacked).
        let board_size = self.board_size(window);
        let page_stacked = self.layout.is_stacked();
        let area = size(
            board_size,
            if page_stacked {
                board_size
            } else {
                self.layout.panel_height_px(board_size)
            },
        );
        let face_size = if vertical {
            size(area.width, (area.height - FACE_GAP) / 2.)
        } else {
            size((area.width - FACE_GAP) / 2., area.height)
        };
        // As large as the face holds "10:00" with its lines above and below.
        let digits = ((face_size.width - px(32.)) / 3.2)
            .min((face_size.height - px(120.)) / 1.2)
            .clamp(px(36.), px(180.));
        let mut face = |side: Side, index: usize, cx: &mut Context<Self>| {
            let running = self.clock.running == Some(side);
            let flagged = self.clock.flagged == Some(side);
            let time = format_clock(self.clock.left(side, now));
            let state = if flagged {
                "Time up"
            } else if running {
                "Your turn"
            } else if self.clock.running.is_some() {
                "Waiting"
            } else {
                "Ready"
            };
            // Red when time is up, and while the last ten seconds run.
            let (background, foreground) = if flagged || (running && widgets::low_time(&time)) {
                (palette.error, palette.bg)
            } else if running {
                (palette.primary, palette.bg)
            } else {
                (palette.elevated, palette.text)
            };
            let moves = self.clock.moves.get(side);
            let rotation = Self::rotation(side, cx);
            if rotation == 0
                && let Some((_, old)) = self.turned.remove(&index)
            {
                cx.drop_image(old, Some(window));
            }
            let turned = (rotation != 0).then(|| {
                let lines = [
                    (format!("Player {index}"), 14., true, 0.75),
                    (time.clone(), 120., true, 1.),
                    (
                        crate::widgets::format_counted(moves as f64, "move", "moves"),
                        14.,
                        false,
                        0.75,
                    ),
                    (state.to_string(), 12., true, 1.),
                ];
                let rgb = Rgba::from(foreground);
                let color = format!(
                    "#{:02x}{:02x}{:02x}",
                    (rgb.r * 255.).round() as u8,
                    (rgb.g * 255.).round() as u8,
                    (rgb.b * 255.).round() as u8
                );
                let key = format!("{lines:?}{color}{rotation}");
                let image = match self.turned.get(&index) {
                    Some((shown, image)) if *shown == key => Some(image.clone()),
                    _ => kchess_board::game_image::rotated_text_image(
                        &lines.map(|(text, size, bold, opacity)| {
                            kchess_board::game_image::TextLine {
                                text,
                                size,
                                bold,
                                opacity,
                            }
                        }),
                        &color,
                        rotation,
                        window.scale_factor(),
                    )
                    .ok()
                    .inspect(|image| {
                        if let Some((_, old)) = self.turned.insert(index, (key, image.clone())) {
                            cx.drop_image(old, Some(window));
                        }
                    }),
                };
                image.map(|image| {
                    img(image)
                        .size_full()
                        .p(px(16.))
                        .object_fit(ObjectFit::Contain)
                })
            });
            let content = v_flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .opacity(0.75)
                        .child(format!("Player {index}")),
                )
                .child(crate::widgets::timer(time.clone(), digits))
                .child(div().text_size(px(14.)).opacity(0.75).child(
                    crate::widgets::format_counted(moves as f64, "move", "moves"),
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(state),
                );
            div()
                .id(("clock-face", index))
                .test_support()
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded(radii::BOX)
                .border_1()
                .border_color(palette.border)
                .bg(background)
                .text_color(foreground)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.press(side, cx)))
                .aria_label(format!(
                    "Player {index} clock, {time}. Press after your move."
                ))
                .map(|face| match turned.flatten() {
                    Some(image) => face.child(image),
                    None => face.child(content),
                })
        };
        let faces = if vertical { v_flex() } else { h_flex() }
            .id("local-clock-faces")
            .w(area.width)
            .h(area.height)
            .flex_none()
            .items_stretch()
            .gap(FACE_GAP)
            .child(face(Side::Top, 1, cx))
            .child(face(Side::Bottom, 2, cx));
        let [top_minutes, bottom_minutes, increment] = &self.clock_inputs;
        let number = |input: &Entity<InputState>, name: &'static str, unit: &'static str| {
            h_flex()
                .gap(px(6.))
                .text_size(px(13.))
                .text_color(palette.text_muted)
                .child(
                    div()
                        .w(px(64.))
                        .child(Input::new(input).small().aria_label(name)),
                )
                .child(unit)
        };
        let row = |title: &'static str, control: AnyElement| {
            widgets::setting_row(widgets::setting_info(title, None, palette), control)
        };
        let rotate = h_flex()
            .gap(px(8.))
            .children(
                [(Side::Top, 1usize), (Side::Bottom, 2)].map(|(side, index)| {
                    let degrees = Self::rotation(side, cx);
                    Button::new(("clock-rotate", index))
                        .small()
                        .outline()
                        .icon(icon("rotate-cw"))
                        .label(format!("{index} · {degrees}°"))
                        .accessibility_label(format!(
                            "Rotate Player {index} clock, currently {degrees} degrees"
                        ))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            Self::rotate(side, cx);
                            cx.notify();
                        }))
                }),
            );
        let running = self.clock.running.is_some();
        let zen = crate::settings::frontend(cx).zen_mode;
        // The shared board's panel's place and size, its actions at the foot as there.
        let panel = widgets::side_panel(palette)
            .id("local-clock-panel")
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(if zen { PANEL_ZEN } else { PANEL_MAX })
            .child(widgets::setting_list(
                [
                    row(
                        "Player 1",
                        number(top_minutes, "Player 1 minutes", "min").into_any_element(),
                    ),
                    row(
                        "Player 2",
                        number(bottom_minutes, "Player 2 minutes", "min").into_any_element(),
                    ),
                    row(
                        "Increment",
                        number(increment, "Increment in seconds", "s").into_any_element(),
                    ),
                    row(
                        "Layout",
                        div()
                            .w(px(160.))
                            .child(
                                Select::new(&self.clock_layout)
                                    .small()
                                    .accessibility_label("Clock layout"),
                            )
                            .into_any_element(),
                    ),
                    row("Rotate", rotate.into_any_element()),
                ],
                palette,
            ))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("clock-reset")
                            .outline()
                            .flex_1()
                            .icon(icon("rotate-ccw"))
                            .label("Reset")
                            .on_click(cx.listener(|this, _, _, cx| this.reset_clock(cx))),
                    )
                    .child(
                        Button::new("clock-start")
                            .primary()
                            .flex_1()
                            .icon(if running { icon("pause") } else { icon("play") })
                            .label(if running { "Pause" } else { "Start" })
                            .tooltip("Space switches turns · P pauses")
                            .on_click(cx.listener(|this, _, _, cx| this.start_or_pause(cx))),
                    ),
            );
        let tabs = mode_tabs(Mode::Clock, palette, cx).into_any_element();
        self.layout
            .render_under("local-clock", tabs, faces, panel, window, cx)
            .into_any_element()
    }
}
