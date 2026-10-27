//! The board editor (`pages/editor.vue`, `EditorBoard.vue`): set up any position, as on Lichess.
//! Drag pieces anywhere (off the board removes them), drag new ones in from the spare rows, or
//! pick a spare piece (or the bin) and click squares; choose the side to move, castling rights
//! and en passant, or type a FEN. Then analyse the position or play it.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::{Color, Piece, Role};

use crate::assets::icon;
use crate::board::looks;
use crate::board::{BoardEvent, BoardState, ChessBoard, EditTool, SparePiece};
use crate::game::{STANDARD_FEN, editor_setup, position_problem, rules};
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, tint};
use crate::voice::control::{BarSpec, Control};
use crate::voice::{Attempt, VoiceEvent};
use crate::widgets::{self, Pill, Tone};

const EMPTY_BOARD: &str = "8/8/8/8/8/8/8/8";
/// The spare rows above and below the board, and the voice bar, take this much of its height.
const SPARE_ROWS: Pixels = px(56.);
const VOICE_BAR: Pixels = px(36.);
const ROLES: [Role; 6] = [
    Role::King,
    Role::Queen,
    Role::Rook,
    Role::Bishop,
    Role::Knight,
    Role::Pawn,
];
const CASTLING: [(&str, &str); 4] = [
    ("K", "White O-O"),
    ("Q", "White O-O-O"),
    ("k", "Black O-O"),
    ("q", "Black O-O-O"),
];

/// `START_SETUP`.
fn start_setup() -> Value {
    json!({
        "board": STANDARD_FEN.split(' ').next().unwrap_or_default(),
        "turn": "white",
        "castling": { "K": true, "Q": true, "k": true, "q": true },
        "ep": "",
    })
}

/// `setupFen`: the position as a FEN, with only the castling rights the pieces allow.
fn setup_fen(setup: &Value) -> String {
    rules("setupFen", std::slice::from_ref(setup))
        .ok()
        .and_then(|fen| fen.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::King => "king",
        Role::Queen => "queen",
        Role::Rook => "rook",
        Role::Bishop => "bishop",
        Role::Knight => "knight",
        Role::Pawn => "pawn",
    }
}

/// The piece a dragged spare shows under the pointer.
struct SpareGhost {
    image: SharedString,
}

impl Render for SpareGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        img(self.image.clone()).size(px(56.))
    }
}

pub struct EditorPage {
    setup: Value,
    orientation: Color,
    tool: EditTool,
    board: Entity<ChessBoard>,
    fen_input: Entity<InputState>,
    fen_error: Option<SharedString>,
    ep_select: Entity<SelectState<Vec<SharedString>>>,
    /// The en passant squares the select offers, after `None`.
    ep_squares: Vec<String>,
    layout: PlayLayout,
    /// The FEN box and the select are refilled on the next frame, where there is a window.
    stale_fields: bool,
    focus: FocusHandle,
    /// “white knight F three”, “remove E four”, “clear board”, “black to move”.
    voice: Control,
    /// A piece said without a colour takes the colour said last.
    last_color: &'static str,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for EditorPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EditorPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> EditorPage {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let fen_input = cx.new(|cx| InputState::new(window, cx));
        let ep_select = cx.new(|cx| SelectState::new(Vec::<SharedString>::new(), None, window, cx));
        let settings = cx.global::<SettingsHandle>().0.clone();
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Edited(placement) = event {
                    this.setup["board"] = json!(placement);
                    this.changed(cx);
                }
            }),
            cx.subscribe_in(&fen_input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.apply_fen(cx);
                }
            }),
            cx.subscribe(
                &ep_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(0, |index| index.row);
                    let ep = row
                        .checked_sub(1)
                        .and_then(|index| this.ep_squares.get(index))
                        .cloned()
                        .unwrap_or_default();
                    this.setup["ep"] = json!(ep);
                    this.changed(cx);
                },
            ),
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.redraw(cx)
            }),
            cx.subscribe_in(
                &crate::voice::handle(cx),
                window,
                |this, _, event: &VoiceEvent, window, cx| this.voice_heard(event, window, cx),
            ),
            cx.observe(&crate::voice::handle(cx), |_, _, cx| cx.notify()),
        ];
        let mut page = EditorPage {
            setup: start_setup(),
            orientation: Color::White,
            tool: EditTool::Pointer,
            board,
            fen_input,
            fen_error: None,
            ep_select,
            ep_squares: Vec::new(),
            layout: PlayLayout::default(),
            stale_fields: true,
            focus: cx.focus_handle(),
            voice: Control::new(true, cx),
            last_color: "white",
            _subscriptions: subscriptions,
        };
        page.redraw(cx);
        page
    }

    /// `editPosition`: the editor on a position (the analysis board's), facing the same way.
    pub fn set_position(
        &mut self,
        fen: &str,
        orientation: Color,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.setup = editor_setup(fen).unwrap_or_else(start_setup);
        self.orientation = orientation;
        self.changed(cx);
    }

    fn fen(&self) -> String {
        setup_fen(&self.setup)
    }

    fn problem(&self) -> Option<&'static str> {
        position_problem(&self.fen())
    }

    fn turn(&self) -> Color {
        if self.setup["turn"] == json!("black") {
            Color::Black
        } else {
            Color::White
        }
    }

    /// The position changed: the board, the FEN box and the en passant choices follow.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.fen_error = None;
        self.stale_fields = true;
        self.redraw(cx);
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let mut state = BoardState::of(
            &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
            self.orientation,
            None,
            None,
        );
        // An unfinished position is no legal one: the board draws the pieces as they stand.
        state.fen = self.fen();
        state.edit = Some(self.tool);
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /// The FEN box shows the position; a valid FEN entered there sets the board.
    fn apply_fen(&mut self, cx: &mut Context<Self>) {
        let text = self.fen_input.read(cx).value().to_string();
        if text.trim() == self.fen() {
            return;
        }
        match editor_setup(&text) {
            Some(setup) => {
                self.setup = setup;
                self.changed(cx);
            }
            None => {
                self.fen_error = Some("That is not a valid FEN.".into());
                cx.notify();
            }
        }
    }

    fn refresh_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fen = self.fen();
        self.fen_input
            .update(cx, |input, cx| input.set_value(fen, window, cx));
        let board = self.setup["board"].clone();
        let turn = self.setup["turn"].clone();
        self.ep_squares = rules("enPassantSquares", &[board, turn])
            .ok()
            .and_then(|squares| serde_json::from_value(squares).ok())
            .unwrap_or_default();
        let items: Vec<SharedString> = std::iter::once(SharedString::from("None"))
            .chain(self.ep_squares.iter().map(|square| square.clone().into()))
            .collect();
        let chosen = self.setup["ep"].as_str().unwrap_or_default();
        let index = self
            .ep_squares
            .iter()
            .position(|square| square == chosen)
            .map_or(0, |index| index + 1);
        self.ep_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_index(Some(IndexPath::new(index)), window, cx);
        });
    }

    fn choose_tool(&mut self, tool: EditTool, cx: &mut Context<Self>) {
        self.tool = tool;
        self.redraw(cx);
    }

    fn set_turn(&mut self, turn: Color, cx: &mut Context<Self>) {
        self.setup["turn"] = json!(if turn == Color::White {
            "white"
        } else {
            "black"
        });
        self.setup["ep"] = json!("");
        self.changed(cx);
    }

    fn start_position(&mut self, cx: &mut Context<Self>) {
        self.setup = start_setup();
        self.changed(cx);
    }

    fn clear_board(&mut self, cx: &mut Context<Self>) {
        self.setup["board"] = json!(EMPTY_BOARD);
        self.setup["ep"] = json!("");
        self.changed(cx);
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        self.orientation = !self.orientation;
        self.redraw(cx);
    }

    fn analyse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.problem().is_some() {
            return;
        }
        let (fen, orientation) = (self.fen(), self.orientation);
        crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
            analysis.analyse_setup(&fen, orientation, cx);
            true
        });
    }

    fn voice_heard(&mut self, event: &VoiceEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.voice.owns(event) || !self.voice.enabled {
            return;
        }
        let heard = match event {
            VoiceEvent::Missed { heard, .. } => {
                let attempt = Attempt {
                    source: "editor",
                    heard,
                    outcome: "unclear",
                    parsed: None,
                    expected: None,
                    fen: None,
                };
                crate::voice::log(&attempt, None, cx);
                return;
            }
            VoiceEvent::Heard { heard, .. } => heard,
        };
        let action = rules("spokenEdit", &[json!(heard.text), json!(self.last_color)])
            .unwrap_or(Value::Null);
        let fen = self.problem().is_none().then(|| self.fen());
        let parsed = action.is_object().then(|| action.to_string());
        let attempt = Attempt {
            source: "editor",
            heard,
            outcome: if action.is_object() {
                "command"
            } else if heard.unclear {
                "unclear"
            } else {
                "invalid"
            },
            parsed: parsed.as_deref(),
            expected: None,
            fen: fen.as_deref(),
        };
        crate::voice::log(&attempt, None, cx);
        let text = |key: &str| action[key].as_str().unwrap_or_default().to_string();
        let side = |color: &str| if color == "white" { "White" } else { "Black" };
        let feedback = match action["kind"].as_str() {
            Some("place") => {
                let (square, color, role) = (text("square"), text("color"), text("role"));
                self.last_color = if color == "white" { "white" } else { "black" };
                let piece = json!({ "color": color, "role": role });
                let board = self.setup["board"].clone();
                if let Ok(board) = rules("withPiece", &[board, json!(square), piece]) {
                    self.setup["board"] = board;
                    self.changed(cx);
                }
                format!("{} {role} on {}.", side(&color), square.to_uppercase())
            }
            Some("remove") => {
                let square = text("square");
                let board = self.setup["board"].clone();
                if let Ok(board) = rules("withPiece", &[board, json!(square), Value::Null]) {
                    self.setup["board"] = board;
                    self.changed(cx);
                }
                format!("Cleared {}.", square.to_uppercase())
            }
            Some("clear") => {
                self.clear_board(cx);
                "Board cleared.".into()
            }
            Some("start") => {
                self.start_position(cx);
                "Starting position.".into()
            }
            Some("turn") => {
                let color = text("color");
                self.set_turn(
                    if color == "white" {
                        Color::White
                    } else {
                        Color::Black
                    },
                    cx,
                );
                format!("{} to move.", side(&color))
            }
            Some("flip") => {
                self.flip(cx);
                "Board flipped.".into()
            }
            Some("analyze") => match self.problem() {
                Some(problem) => format!("Can’t analyse yet: {problem}"),
                None => {
                    self.analyse(window, cx);
                    String::new()
                }
            },
            _ if heard.unclear => "Didn’t catch that. Try “white knight F three”.".into(),
            _ => format!(
                "Heard “{}”. Try “white knight F three”, “remove E four” or “clear board”.",
                heard.text
            ),
        };
        self.voice.feedback = (!feedback.is_empty()).then(|| feedback.into());
        cx.notify();
    }

    /// A spare row: the move tool, the six pieces of one side, and the bin.
    fn render_spares(
        &self,
        color: Color,
        set: &str,
        width: Pixels,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement {
        // `.spare`: eight cells across the row (4px padding and gaps, a 1px border), square up to
        // 56px tall.
        let across = (width - px(10.) - px(4.) * 7.) / 8.;
        let cell = across.min(SPARE_ROWS);
        let tool_button = |id: &'static str,
                           tool: EditTool,
                           glyph: gpui_kit::component::Icon,
                           label: &'static str,
                           cx: &mut Context<Self>| {
            let selected = self.tool == tool;
            div()
                .id(SharedString::from(format!("{id}-{color:?}")))
                .flex_1()
                .h(cell)
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .text_color(if selected {
                    palette.text
                } else {
                    palette.text_muted
                })
                .cursor_pointer()
                .when(selected, |spare| {
                    spare
                        .bg(tint(palette.primary, 30.))
                        .border_2()
                        .border_color(palette.primary)
                })
                .when(!selected, |spare| {
                    spare.hover(|spare| spare.bg(palette.accented))
                })
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.choose_tool(tool, cx)))
                .child(glyph.size(px(20.)))
        };
        let pieces = ROLES.map(|role| {
            let piece = Piece { color, role };
            let selected = self.tool == EditTool::Place(piece);
            let image = SharedString::from(looks::piece_path(set, piece));
            let ghost = image.clone();
            let label = format!(
                "{} {}",
                if color == Color::White {
                    "White"
                } else {
                    "Black"
                },
                role_name(role)
            );
            div()
                .id(SharedString::from(format!("spare-{color:?}-{role:?}")))
                .flex_1()
                .h(cell)
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .when(selected, |spare| {
                    spare
                        .bg(tint(palette.primary, 30.))
                        .border_2()
                        .border_color(palette.primary)
                })
                .when(!selected, |spare| {
                    spare.hover(|spare| spare.bg(palette.accented))
                })
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                })
                // Pressing a spare piece chooses it, and dragging it puts a new one on the board.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.choose_tool(EditTool::Place(piece), cx)),
                )
                .on_drag(SparePiece(piece), move |_, _, _, cx| {
                    cx.new(|_| SpareGhost {
                        image: ghost.clone(),
                    })
                })
                .child(
                    // A stable light square keeps both piece colors legible in every theme.
                    // `.spare-piece`: 86% of the cell, the piece contained in it.
                    h_flex()
                        .w(across * 0.86)
                        .h(cell * 0.86)
                        .justify_center()
                        .rounded(px(5.))
                        .bg(rgb(0xf0d9b5))
                        .child(img(image).size(cell * 0.86)),
                )
                .into_any_element()
        });
        h_flex()
            .gap(px(4.))
            .p(px(4.))
            .rounded(px(10.))
            .border_1()
            .border_color(palette.border)
            .bg(palette.elevated)
            .child(tool_button(
                "move-pieces",
                EditTool::Pointer,
                icon("mouse-pointer-2"),
                "Move pieces",
                cx,
            ))
            .children(pieces)
            .child(tool_button(
                "remove-pieces",
                EditTool::Trash,
                icon("trash-2"),
                "Remove pieces",
                cx,
            ))
    }
}

impl Render for EditorPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.stale_fields) {
            self.refresh_fields(window, cx);
        }
        let palette = *cx.global::<Palette>();
        let set = looks::piece_set(
            &cx.global::<SettingsHandle>()
                .0
                .read(cx)
                .settings
                .frontend
                .piece_set,
        )
        .to_string();
        let problem = self.problem();
        let turn = self.turn();
        let available: Value =
            rules("castlingAvailable", &[self.setup["board"].clone()]).unwrap_or(Value::Null);
        let board_size = self
            .layout
            .board_size_in(px(0.), SPARE_ROWS + VOICE_BAR, window);
        let (top, bottom) = if self.orientation == Color::White {
            (Color::Black, Color::White)
        } else {
            (Color::White, Color::Black)
        };
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(px(8.))
            .child(self.render_spares(top, &set, board_size, cx, &palette))
            .child(crate::board::cached(&self.board))
            .child(self.render_spares(bottom, &set, board_size, cx, &palette))
            // Voice sits at the end of its row, as in `BoardEditor.vue`.
            .child(h_flex().justify_end().child(self.voice.bar(
                cx.entity().downgrade(),
                |this: &mut Self| &mut this.voice,
                BarSpec {
                    id: "editor-voice",
                    hint: "Say “white knight F three”, “remove E four” or “clear board”.",
                    active: true,
                    grammar: crate::voice::grammar("EDITOR_GRAMMAR"),
                },
                cx,
            )));
        let label = |text: &'static str| {
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.text_muted)
                .child(text)
        };
        let castling = CASTLING.map(|(key, text)| {
            let allowed = available[key] == json!(true);
            Checkbox::new(SharedString::from(format!("castling-{key}")))
                .label(text)
                .checked(self.setup["castling"][key] == json!(true) && allowed)
                .disabled(!allowed)
                .when(!allowed, |checkbox| {
                    checkbox.tooltip("The king or rook is not on its starting square")
                })
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    this.setup["castling"][key] = json!(*checked);
                    this.changed(cx);
                }))
                .into_any_element()
        });
        let [k, q, bk, bq] = castling;
        let fen_box = v_flex()
            .gap(px(6.))
            .child(label("FEN"))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div().flex_1().min_w_0().child(
                            // `.fen-input`: 12px monospace, set on the input, whose size
                            // would otherwise make it 14px.
                            Input::new(&self.fen_input)
                                .small()
                                .text_size(px(12.))
                                .font_family("Menlo")
                                .aria_label("FEN"),
                        ),
                    )
                    .child(
                        Button::new("copy-fen")
                            .outline()
                            .small()
                            .icon(icon("copy"))
                            .tooltip("Copy FEN")
                            .accessibility_label("Copy FEN")
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.fen()));
                                window.push_notification("FEN copied", cx);
                            })),
                    ),
            )
            .children(self.fen_error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }));
        let panel = widgets::side_panel(&palette)
            .h(self
                .layout
                .fixed(self.layout.panel_height_px(board_size) + SPARE_ROWS))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(16.))
            .child(
                widgets::StatusBanner::new(
                    if problem.is_some() {
                        Tone::Alert
                    } else {
                        Tone::Plain
                    },
                    if problem.is_some() {
                        icon("triangle-alert")
                    } else {
                        icon("circle-check")
                    },
                    if problem.is_some() {
                        "Not a legal position yet".into()
                    } else {
                        "Ready to analyse".into()
                    },
                )
                .detail(
                    problem
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!(
                                "{} to move",
                                if turn == Color::White {
                                    "White"
                                } else {
                                    "Black"
                                }
                            )
                        })
                        .into(),
                ),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(label("Side to move"))
                    .child(widgets::pills(
                        "editor-turn",
                        ["White", "Black"]
                            .into_iter()
                            .map(|text| Pill {
                                icon: None,
                                label: text.into(),
                                badge: None,
                                disabled: false,
                            })
                            .collect(),
                        usize::from(turn == Color::Black),
                        {
                            let page = cx.entity().downgrade();
                            move |index, _, cx| {
                                let turn = if index == 1 {
                                    Color::Black
                                } else {
                                    Color::White
                                };
                                let _ = page.update(cx, |page, cx| page.set_turn(turn, cx));
                            }
                        },
                        false,
                        &palette,
                    )),
            )
            .child(
                v_flex().gap(px(6.)).child(label("Castling")).child(
                    v_flex()
                        .gap(px(6.))
                        .text_size(px(13.))
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .child(div().flex_1().child(k))
                                .child(div().flex_1().child(q)),
                        )
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .child(div().flex_1().child(bk))
                                .child(div().flex_1().child(bq)),
                        ),
                ),
            )
            .when(!self.ep_squares.is_empty(), |panel| {
                panel.child(
                    v_flex().gap(px(6.)).child(label("En passant")).child(
                        div().w(px(128.)).child(
                            Select::new(&self.ep_select)
                                .small()
                                .accessibility_label("En passant"),
                        ),
                    ),
                )
            })
            .child(fen_box)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("start-position")
                            .outline()
                            .icon(icon("rotate-ccw"))
                            .label("Starting position")
                            .on_click(cx.listener(|this, _, _, cx| this.start_position(cx))),
                    )
                    .child(
                        Button::new("clear-board")
                            .outline()
                            .icon(icon("eraser"))
                            .label("Clear board")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_board(cx))),
                    )
                    .child(
                        Button::new("flip-board")
                            .outline()
                            .icon(icon("arrow-up-down"))
                            .label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
                    ),
            )
            .child(
                v_flex()
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .gap(px(8.))
                    .child(
                        Button::new("analyse-position")
                            .primary()
                            .w_full()
                            .icon(icon("microscope"))
                            .label("Analyse this position")
                            .disabled(problem.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.analyse(window, cx))),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Button::new("play-stockfish")
                                    .outline()
                                    .flex_1()
                                    .icon(icon("cpu"))
                                    .label("Play Stockfish")
                                    .disabled(problem.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        crate::shell::play_computer_from(this.fen(), window, cx)
                                    })),
                            )
                            .child(
                                Button::new("play-friend")
                                    .outline()
                                    .flex_1()
                                    .icon(icon("users-round"))
                                    .label("Play a friend here")
                                    .disabled(problem.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        crate::shell::play_local_from(this.fen(), window, cx)
                                    })),
                            ),
                    ),
            );
        div()
            .size_full()
            .track_focus(&self.focus)
            .child(
                self.layout
                    .render("editor", board_column, panel, window, cx),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{setup_fen, start_setup};
    use serde_json::json;

    #[test]
    fn a_setup_reads_as_its_fen() {
        assert_eq!(
            setup_fen(&start_setup()),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        // Castling rights the pieces no longer allow are left out.
        let mut setup = start_setup();
        setup["board"] = json!("4k3/8/8/8/8/8/8/4K3");
        setup["turn"] = json!("black");
        assert_eq!(setup_fen(&setup), "4k3/8/8/8/8/8/8/4K3 b - - 0 1");
    }
}
