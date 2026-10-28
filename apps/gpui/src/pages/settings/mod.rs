//! Settings (`pages/settings.vue`): one category at a time, picked in the sidebar while Settings is
//! open (`SETTINGS_SECTIONS`). Every change saves itself (`settings.rs`).

mod accounts;
mod data;
mod engine;
mod offline;
pub mod updates;
mod voice;
mod voice_history;

use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, IndexPath, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{
    AppTheme, Appearance, CoordinateMode, EngineLevel, EngineStatus, PieceAnimation, PromotionMode,
    Settings,
};
use serde::Deserialize;
use shakmaty::{Color, Piece, Role};

use crate::assets::icon;
use crate::board::looks;
use crate::game::rules;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;
use crate::widgets::{self, columns};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Themes,
    Gameplay,
    Gestures,
    Online,
    Analysis,
    Sound,
    Notifications,
    Voice,
    Engine,
    Accounts,
    Offline,
    Data,
    Updates,
}

/// The categories the sidebar lists while Settings is open, in the Electron app's order.
pub fn sections() -> [(Section, &'static str, Icon); 14] {
    [
        (Section::Appearance, "Appearance", icon("palette")),
        (Section::Themes, "App theme", icon("swatch-book")),
        (Section::Gameplay, "Gameplay", icon("gamepad-2")),
        (Section::Gestures, "Gestures", icon("hand")),
        (Section::Online, "Online play", icon("globe-2")),
        (Section::Analysis, "Analysis", icon("microscope")),
        (Section::Sound, "Sound", icon("volume-2")),
        (Section::Notifications, "Notifications", icon("bell")),
        (Section::Voice, "Voice input", icon("mic")),
        (Section::Engine, "Chess engine", icon("cpu")),
        (Section::Accounts, "My accounts", icon("user-round")),
        (Section::Offline, "Offline downloads", icon("download")),
        (Section::Data, "Data & storage", icon("database")),
        (Section::Updates, "Updates", icon("download")),
    ]
}

/// A yes/no row: its id, title, hint, state and how it changes the settings.
type SwitchRow = (
    &'static str,
    &'static str,
    Option<&'static str>,
    bool,
    fn(&mut Settings, bool),
);

/// `pollItems`: minutes between correspondence checks.
const POLL: [(f64, &str); 6] = [
    (0., "Off"),
    (2., "2 min"),
    (5., "5 min"),
    (15., "15 min"),
    (30., "30 min"),
    (60., "1 hour"),
];

fn poll_row(minutes: f64) -> Option<IndexPath> {
    POLL.iter()
        .position(|(choice, _)| *choice == minutes)
        .map(IndexPath::new)
}

/// The theme pickers' and piece pickers' columns: `minmax(92px, 1fr)`, 8px apart.
const OPTION_MIN: Pixels = px(92.);
const APP_THEME_MIN: Pixels = px(112.);
const OPTION_GAP: Pixels = px(8.);
/// Below this page width each row puts its control under its title.
const NARROW: Pixels = px(640.);

/// A rung of the engine ladder, as Settings lists it.
#[derive(Deserialize)]
struct Rung {
    id: EngineLevel,
    label: String,
    elo: Option<f64>,
}

pub struct SettingsPage {
    section: Section,
    area: Measured,
    volume: Entity<SliderState>,
    poll: Entity<SelectState<Vec<SharedString>>>,
    /// Interface size: Auto (its percent here) and the steps; refilled when either changes.
    zoom: Entity<SelectState<Vec<SharedString>>>,
    zoom_shown: Option<(crate::interface_zoom::Setting, u16)>,
    ladder: Vec<Rung>,
    default_levels: Vec<EngineLevel>,
    /// The engine the core finds (`engineInfo`), and whether it is being downloaded or deleted.
    engine: Option<EngineStatus>,
    engine_busy: bool,
    /// What the notification test says, and whether it is counting down.
    test_note: Option<SharedString>,
    test_pending: bool,
    /// The microphone test, and why it could not start.
    mic_test: Option<voice::MicTest>,
    mic_error: Option<SharedString>,
    mic_access: crate::microphone::Access,
    mic_asking: bool,
    /// Data & storage: what clearing the scores and exporting diagnostics said.
    runs_note: Option<SharedString>,
    diagnostics_note: Option<SharedString>,
    exporting: bool,
    voice_history: Option<Entity<voice_history::VoiceHistory>>,
    /// The offline speech model, and whether this page is downloading it.
    voice_model: Option<kchess_core::voice::Status>,
    voice_model_error: Option<SharedString>,
    voice_downloading: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> SettingsPage {
        let handle = cx.global::<SettingsHandle>().0.clone();
        let (volume_now, poll_now) = {
            let settings = &handle.read(cx).settings;
            (
                settings.frontend.sound_volume,
                settings.core.correspondence_poll,
            )
        };
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.05)
                .default_value(volume_now as f32)
        });
        let poll = cx.new(|cx| {
            SelectState::new(
                POLL.iter()
                    .map(|(_, label)| SharedString::from(*label))
                    .collect(),
                poll_row(poll_now),
                window,
                cx,
            )
        });
        let zoom = cx.new(|cx| SelectState::new(Vec::<SharedString>::new(), None, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &zoom,
                window,
                |this, state, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    use crate::interface_zoom::{PERCENTS, Setting, choose};
                    let row = state.read(cx).selected_index(cx).map(|index| index.row);
                    let setting = match row {
                        Some(0) | None => Setting::Auto,
                        Some(row) => PERCENTS
                            .get(row - 1)
                            .map_or(Setting::Auto, |p| Setting::Percent(*p)),
                    };
                    choose(setting, window, cx);
                    this.zoom_shown = None;
                    cx.notify();
                },
            ),
            // Back from System Settings: the microphone permission may have changed there.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() && this.section == Section::Voice {
                    this.refresh_microphone(cx);
                }
            }),
            cx.subscribe(&volume, |_, _, event: &SliderEvent, cx| {
                let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                // The slider's steps are f32: keep the hundredths the Electron slider stores.
                let volume = (f64::from(value.start()) * 100.).round() / 100.;
                crate::settings::change(cx, move |s| s.frontend.sound_volume = volume);
            }),
            cx.subscribe_in(
                &poll,
                window,
                |_, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let row = state.read(cx).selected_index(cx).map(|index| index.row);
                    if let Some(&(minutes, _)) = row.and_then(|row| POLL.get(row)) {
                        crate::settings::change(cx, move |s| s.core.correspondence_poll = minutes);
                    }
                },
            ),
            // The background review's progress and the puzzle database show as they change.
            cx.observe(
                &cx.global::<crate::reviews::ReviewHandle>().0.clone(),
                |_, _, cx| cx.notify(),
            ),
            cx.observe(
                &cx.global::<crate::puzzle_db::PuzzleDbHandle>().0.clone(),
                |_, _, cx| cx.notify(),
            ),
            cx.observe(
                &cx.global::<crate::updates::UpdatesHandle>().0.clone(),
                |_, _, cx| cx.notify(),
            ),
            cx.observe(&crate::accounts::handle(cx), |_, _, cx| cx.notify()),
            // Settings changed elsewhere (the other app, a reload) show in the controls too.
            cx.subscribe_in(
                &handle,
                window,
                |this, handle, _: &SettingsChanged, window, cx| {
                    let (volume, poll) = {
                        let settings = &handle.read(cx).settings;
                        (
                            settings.frontend.sound_volume,
                            settings.core.correspondence_poll,
                        )
                    };
                    this.volume.update(cx, |state, cx| {
                        if (f64::from(state.value().start()) - volume).abs() > 0.001 {
                            state.set_value(volume as f32, window, cx);
                        }
                    });
                    this.poll.update(cx, |state, cx| {
                        if state.selected_index(cx) != poll_row(poll) {
                            state.set_selected_index(poll_row(poll), window, cx);
                        }
                    });
                    cx.notify();
                },
            ),
        ];
        let ladder = rules("engineLadder", &[])
            .ok()
            .and_then(|ladder| serde_json::from_value(ladder).ok())
            .unwrap_or_default();
        let default_levels = rules("defaultEngineLevels", &[])
            .ok()
            .and_then(|levels| serde_json::from_value(levels).ok())
            .unwrap_or_default();
        SettingsPage {
            section: Section::Appearance,
            area: Measured::default(),
            volume,
            poll,
            zoom,
            zoom_shown: None,
            ladder,
            default_levels,
            engine: None,
            engine_busy: false,
            test_note: None,
            test_pending: false,
            mic_test: None,
            mic_error: None,
            mic_access: crate::microphone::status(),
            mic_asking: false,
            runs_note: None,
            diagnostics_note: None,
            exporting: false,
            voice_history: None,
            voice_model: None,
            voice_model_error: None,
            voice_downloading: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// Show one category, from the top (`jumpToSection`).
    pub fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        if section != Section::Voice {
            self.stop_mic_test(cx);
        }
        self.section = section;
        if section == Section::Voice {
            self.refresh_microphone(cx);
            match &self.voice_history {
                Some(history) => history.update(cx, |history, cx| history.load(cx)),
                None => {
                    self.voice_history = Some(cx.new(voice_history::VoiceHistory::new));
                }
            }
        }
        if matches!(section, Section::Engine | Section::Offline) {
            self.refresh_engine(cx);
        }
        if matches!(section, Section::Accounts | Section::Data) {
            crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        }
        if section == Section::Offline {
            self.voice_model_error = None;
            self.refresh_voice_model(cx);
            let db = cx.global::<crate::puzzle_db::PuzzleDbHandle>().0.clone();
            db.update(cx, |db, cx| db.refresh(cx));
        }
        cx.notify();
    }

    /// The Interface size select shows the setting, and what Auto means on this display.
    fn sync_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::interface_zoom::{PERCENTS, Setting, auto_for};
        let shown = (
            cx.global::<crate::interface_zoom::InterfaceZoom>().setting,
            auto_for(window, cx),
        );
        if self.zoom_shown == Some(shown) {
            return;
        }
        self.zoom_shown = Some(shown);
        let (setting, auto) = shown;
        let items: Vec<SharedString> = std::iter::once(format!("Auto ({auto}%)").into())
            .chain(PERCENTS.iter().map(|percent| format!("{percent}%").into()))
            .collect();
        let row = match setting {
            Setting::Auto => 0,
            Setting::Percent(percent) => PERCENTS
                .iter()
                .position(|step| *step == percent)
                .map_or(0, |index| index + 1),
        };
        self.zoom.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_index(Some(IndexPath::new(row)), window, cx);
        });
    }

    /// The width inside a category's card, for the pickers' columns.
    fn inner_width(&self, window: &Window) -> Pixels {
        let page = self.area.size().width;
        let page = if page > px(0.) {
            page
        } else {
            window.viewport_size().width
        };
        // The page's sides, the card's padding and border, and the row's own padding.
        page - side_padding(window) * 2. - px(42.) - px(8.)
    }

    fn narrow(&self) -> bool {
        let width = self.area.size().width;
        width > px(0.) && width < NARROW
    }
}

/// A row's title and its optional hint (`.setting-info`).
fn info(title: impl Into<SharedString>, hint: Option<SharedString>, palette: &Palette) -> Div {
    v_flex()
        .gap(px(3.))
        .min_w_0()
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.highlighted)
                .child(title.into()),
        )
        .children(hint.map(|hint| {
            div()
                .text_size(px(12.))
                .line_height(relative(1.45))
                .text_color(palette.text_muted)
                .child(hint)
        }))
}

/// The category's rows (`.settings-list`), with dividers between them.
struct Rows {
    narrow: bool,
    rows: Vec<AnyElement>,
}

impl Rows {
    fn new(narrow: bool) -> Rows {
        Rows {
            narrow,
            rows: Vec::new(),
        }
    }

    /// `.setting-row`: the title on the left (up to 420px), the control beside it (up to 520px).
    fn row(
        self,
        title: impl Into<SharedString>,
        hint: Option<SharedString>,
        control: impl IntoElement,
        palette: &Palette,
    ) -> Rows {
        self.row_with(info(title, hint, palette), control)
    }

    /// A row whose title column says more than a title and a hint (a level meter).
    fn row_with(mut self, info: Div, control: impl IntoElement) -> Rows {
        let control = div().w_full().max_w(px(520.)).child(control);
        let row = if self.narrow {
            v_flex()
                .gap(px(10.))
                .child(info)
                .child(control)
                .into_any_element()
        } else {
            h_flex()
                .gap(px(40.))
                .child(info.w(px(420.)).min_w(px(200.)).flex_shrink(1.))
                .child(div().flex_1().min_w_0().child(control))
                .into_any_element()
        };
        self.rows.push(row);
        self
    }

    /// A yes/no row: the switch sits at the start of the control column.
    #[expect(
        clippy::too_many_arguments,
        reason = "one argument per part of the row"
    )]
    fn switch(
        self,
        id: &'static str,
        title: &'static str,
        hint: Option<&'static str>,
        on: bool,
        enabled: bool,
        change: impl Fn(&mut Settings, bool) + 'static,
        palette: &Palette,
    ) -> Rows {
        let change = Rc::new(change);
        let control = h_flex().child(
            Switch::new(id)
                .checked(on)
                .disabled(!enabled)
                .accessibility_label(title)
                .on_click(move |checked, _, cx| {
                    let (checked, change) = (*checked, Rc::clone(&change));
                    crate::settings::change(cx, move |settings| change(settings, checked));
                }),
        );
        self.row(title, hint.map(SharedString::from), control, palette)
    }

    /// `.setting-row.stacked`: a control that needs the full width goes under its title.
    fn stacked(
        mut self,
        title: &'static str,
        hint: Option<&'static str>,
        control: impl IntoElement,
        palette: &Palette,
    ) -> Rows {
        self.rows.push(
            v_flex()
                .gap(px(10.))
                .child(info(title, hint.map(SharedString::from), palette))
                .child(control)
                .into_any_element(),
        );
        self
    }

    fn card(self, palette: &Palette) -> Div {
        self.card_under(None, palette)
    }

    /// The rows in a card that opens with `header` (`.card-header`: a title and its status).
    fn card_under(self, header: Option<AnyElement>, palette: &Palette) -> Div {
        // `.settings-list` keeps 4px above and below: the rows bring their own padding.
        widgets::card(palette)
            .py(px(4.))
            .children(header.map(|header| div().pt(px(14.)).pb(px(4.)).px(px(4.)).child(header)))
            .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .py(px(18.))
                    .px(px(4.))
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(palette.border)
                    })
                    .child(row)
            }))
    }
}

/// A segmented choice (`UTabs variant="pill"`) that changes one setting.
fn choice<T: Copy + PartialEq + 'static>(
    id: &'static str,
    items: &[(T, &'static str, Option<Icon>)],
    current: T,
    set: impl Fn(&mut Settings, T) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    let values: Vec<T> = items.iter().map(|(value, _, _)| *value).collect();
    let selected = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    widgets::pill_tabs(
        id,
        items
            .iter()
            .map(|(_, label, glyph)| (glyph.clone(), SharedString::from(*label)))
            .collect(),
        selected,
        move |index, _, cx| {
            if let Some(&value) = values.get(index) {
                let set = set.clone();
                crate::settings::change(cx, move |settings| set(settings, value));
            }
        },
        palette,
    )
}

/// `.theme-option`: a picture and its name; the chosen one is ringed in the primary color.
#[expect(
    clippy::too_many_arguments,
    reason = "one argument per part of the option"
)]
fn option(
    id: impl Into<ElementId>,
    width: Pixels,
    chosen: bool,
    picture: impl IntoElement,
    name: impl Into<SharedString>,
    tag: Option<&'static str>,
    choose: impl Fn(&mut App) + 'static,
    palette: &Palette,
) -> impl IntoElement {
    let name = name.into();
    v_flex()
        .id(id)
        .role(gpui_kit::Role::Button)
        .aria_label(name.clone())
        .aria_selected(chosen)
        .w(width)
        .items_center()
        .gap(px(5.))
        .py(px(6.))
        .px(px(5.))
        .rounded(px(10.))
        .bg(palette.bg)
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(chosen, |option| {
            // The border and its 1px outline.
            option
                .border_2()
                .border_color(palette.primary)
                .py(px(5.))
                .px(px(4.))
                .text_color(palette.highlighted)
        })
        .when(!chosen, |option| {
            option
                .border_1()
                .border_color(palette.border)
                .text_color(palette.text_muted)
                .hover(|option| {
                    option
                        .border_color(palette.border_accented)
                        .bg(palette.accented)
                        .text_color(palette.toned)
                })
        })
        .on_click(move |_, _, cx| choose(cx))
        .child(picture)
        .child(name)
        .children(tag.map(|tag| {
            div()
                .text_size(px(9.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.primary)
                .child(tag.to_uppercase())
        }))
}

/// Lay pickers out in rows of `count`, `OPTION_GAP` apart (`.theme-grid`).
fn grid(count: usize, options: Vec<AnyElement>) -> impl IntoElement {
    let mut rows = Vec::new();
    let mut options = options.into_iter().peekable();
    while options.peek().is_some() {
        rows.push(
            h_flex()
                .gap(OPTION_GAP)
                .children(options.by_ref().take(count)),
        );
    }
    v_flex().gap(OPTION_GAP).children(rows)
}

/// The inner width of a picker of `width`: its padding and border.
fn picture_width(width: Pixels) -> Pixels {
    width - px(12.)
}

fn piece_preview(set: &str, width: Pixels) -> impl IntoElement {
    let side = picture_width(width) / 2.;
    let square = |piece: Piece, color: u32| {
        div().size(side).bg(rgb(color)).child(
            img(SharedString::from(looks::piece_path(set, piece)))
                .size(side)
                .object_fit(ObjectFit::Cover),
        )
    };
    h_flex()
        .rounded(px(6.))
        .overflow_hidden()
        .child(square(
            Piece {
                color: Color::White,
                role: Role::Knight,
            },
            0xf0d9b5,
        ))
        .child(square(
            Piece {
                color: Color::Black,
                role: Role::Queen,
            },
            0xb58863,
        ))
}

fn board_swatch(image: &str, width: Pixels) -> impl IntoElement {
    img(SharedString::from(format!("boards/{image}")))
        .size(picture_width(width))
        .rounded(px(6.))
        .object_fit(ObjectFit::Cover)
}

/// `previewColors`: the theme's background, a raised card, its text and primary colors.
fn theme_preview(theme: &str, custom: &[AppTheme], dark: bool, width: Pixels) -> impl IntoElement {
    let colors = crate::theme::colors(theme, custom, dark);
    let color = |name: &str| {
        colors
            .get(name)
            .and_then(|value| value.as_str())
            .map(looks::hex)
            .unwrap_or_default()
    };
    let width = picture_width(width);
    let card_width = (width - px(14.)) * 0.7;
    h_flex()
        .w(width)
        .h(width * 10. / 16.)
        .items_end()
        .p(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(color("--ui-border"))
        .bg(color("--ui-bg"))
        .child(
            v_flex()
                .w(card_width)
                .gap(px(3.))
                .p(px(5.))
                .rounded(px(4.))
                .bg(color("--ui-bg-elevated"))
                .child(
                    div()
                        .h(px(4.))
                        .w((card_width - px(10.)) * 0.8)
                        .rounded(px(2.))
                        .bg(color("--ui-text").opacity(0.75)),
                )
                .child(
                    div()
                        .h(px(4.))
                        .w((card_width - px(10.)) * 0.45)
                        .rounded(px(2.))
                        .bg(color("--ui-primary")),
                ),
        )
}

/// `allThemes`: the default look, the presets, then the user's own (replacing a preset's id).
fn theme_list(custom: &[AppTheme]) -> Vec<AppTheme> {
    let mut themes = vec![AppTheme {
        id: "kchess".into(),
        name: "KChess".into(),
        light: None,
        dark: None,
        custom: None,
    }];
    themes.extend(
        serde_json::from_value::<Vec<AppTheme>>(kchess_domain::theme::presets().clone())
            .unwrap_or_default(),
    );
    for theme in custom {
        let theme = AppTheme {
            custom: Some(true),
            ..theme.clone()
        };
        match themes.iter_mut().find(|known| known.id == theme.id) {
            Some(known) => *known = theme,
            None => themes.push(theme),
        }
    }
    themes
}

fn appearance_items() -> [(Appearance, &'static str, Option<Icon>); 3] {
    [
        (Appearance::System, "System", Some(icon("monitor"))),
        (Appearance::Light, "Light", Some(icon("sun"))),
        (Appearance::Dark, "Dark", Some(icon("moon"))),
    ]
}

impl SettingsPage {
    fn appearance(&self, settings: &Settings, window: &Window, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        let width = self.inner_width(window);
        let (count, item) = columns(width, OPTION_MIN, OPTION_GAP);
        let pieces = looks::piece_sets()
            .iter()
            .map(|set| {
                let id = set.id.clone();
                option(
                    SharedString::from(format!("piece-set-{}", set.id)),
                    item,
                    frontend.piece_set == set.id,
                    piece_preview(&set.id, item),
                    set.name.clone(),
                    None,
                    move |cx| {
                        let id = id.clone();
                        crate::settings::change(cx, move |s| s.frontend.piece_set = id);
                    },
                    palette,
                )
                .into_any_element()
            })
            .collect();
        let boards = looks::boards()
            .iter()
            .map(|board| {
                let id = board.id.clone();
                option(
                    SharedString::from(format!("board-{}", board.id)),
                    item,
                    frontend.board_theme == board.id,
                    board_swatch(&board.image, item),
                    board.name.clone(),
                    None,
                    move |cx| {
                        let id = id.clone();
                        crate::settings::change(cx, move |s| s.frontend.board_theme = id);
                    },
                    palette,
                )
                .into_any_element()
            })
            .collect();
        Rows::new(self.narrow())
            .row(
                "Theme",
                None,
                choice(
                    "appearance",
                    &appearance_items(),
                    frontend.appearance,
                    |s, value| s.frontend.appearance = value,
                    palette,
                ),
                palette,
            )
            .row(
                "Interface size",
                Some(
                    if cfg!(target_os = "macos") {
                        "⌘ + and ⌘ − adjust it anywhere."
                    } else {
                        "Ctrl + and Ctrl − adjust it anywhere."
                    }
                    .into(),
                ),
                Select::new(&self.zoom).accessibility_label("Interface size"),
                palette,
            )
            .row(
                "Board coordinates",
                None,
                choice(
                    "coordinates",
                    &[
                        (CoordinateMode::Inside, "Inside", None),
                        (CoordinateMode::Outside, "Outside", None),
                        (CoordinateMode::None, "Hidden", None),
                    ],
                    frontend.coordinates,
                    |s, value| s.frontend.coordinates = value,
                    palette,
                ),
                palette,
            )
            .row(
                "Piece animation",
                None,
                choice(
                    "animation",
                    &[
                        (PieceAnimation::None, "Off", None),
                        (PieceAnimation::Fast, "Fast", None),
                        (PieceAnimation::Normal, "Normal", None),
                        (PieceAnimation::Slow, "Slow", None),
                    ],
                    frontend.piece_animation,
                    |s, value| s.frontend.piece_animation = value,
                    palette,
                ),
                palette,
            )
            .stacked("Piece set", None, grid(count, pieces), palette)
            .stacked("Board theme", None, grid(count, boards), palette)
            .card(palette)
    }

    fn themes(
        &self,
        settings: &Settings,
        custom: &[AppTheme],
        dir: &str,
        problems: &[String],
        window: &Window,
        palette: &Palette,
    ) -> Div {
        let frontend = &settings.frontend;
        let (count, item) = columns(self.inner_width(window), APP_THEME_MIN, OPTION_GAP);
        let themes = theme_list(custom);
        let picker = |dark: bool| {
            let chosen = if dark {
                &frontend.dark_theme
            } else {
                &frontend.light_theme
            };
            let options = themes
                .iter()
                .map(|theme| {
                    let id = theme.id.clone();
                    option(
                        SharedString::from(format!(
                            "{}-theme-{}",
                            if dark { "dark" } else { "light" },
                            theme.id
                        )),
                        item,
                        *chosen == theme.id,
                        theme_preview(&theme.id, custom, dark, item),
                        theme.name.clone(),
                        (theme.custom == Some(true)).then_some("custom"),
                        move |cx| {
                            let id = id.clone();
                            crate::settings::change(cx, move |s| {
                                if dark {
                                    s.frontend.dark_theme = id;
                                } else {
                                    s.frontend.light_theme = id;
                                }
                            });
                        },
                        palette,
                    )
                    .into_any_element()
                })
                .collect();
            grid(count, options)
        };
        let dir = dir.to_string();
        let own = v_flex()
            .gap(px(10.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        Button::new("open-themes")
                            .outline()
                            .icon(icon("folder-open"))
                            .label("Open themes folder")
                            .tooltip(SharedString::from(format!(
                                "Copy _template.json there, edit its colors, then reload. {dir}"
                            )))
                            .on_click(move |_, _, cx| cx.open_with_system(Path::new(&dir))),
                    )
                    .child(
                        Button::new("reload-themes")
                            .outline()
                            .icon(icon("refresh-cw"))
                            .label("Reload themes")
                            .on_click(|_, _, cx| {
                                let handle = cx.global::<SettingsHandle>().0.clone();
                                crate::settings::reload(&handle, cx);
                            }),
                    ),
            )
            .children(problems.iter().map(|problem| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.warning)
                    .child(format!("• {problem}"))
            }));
        Rows::new(self.narrow())
            .row(
                "Appearance",
                None,
                choice(
                    "theme-appearance",
                    &appearance_items(),
                    frontend.appearance,
                    |s, value| s.frontend.appearance = value,
                    palette,
                ),
                palette,
            )
            .stacked("Light mode theme", None, picker(false), palette)
            .stacked("Dark mode theme", None, picker(true), palette)
            .stacked("Your own themes", None, own, palette)
            .card(palette)
    }

    fn gameplay(&self, settings: &Settings, window: &Window, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        let levels = settings.core.engine_levels.clone();
        let ladder_ids: Vec<EngineLevel> = self.ladder.iter().map(|rung| rung.id).collect();
        // `.engine-levels`: `repeat(auto-fill, minmax(12rem, 1fr))`, 12px by 24px apart.
        let (count, width) = columns(self.inner_width(window), px(192.), px(24.));
        let checkboxes =
            h_flex()
                .flex_wrap()
                .gap_x(px(24.))
                .gap_y(px(12.))
                .children(self.ladder.iter().map(|rung| {
                    let on = levels.contains(&rung.id);
                    let id = rung.id;
                    let ladder_ids = ladder_ids.clone();
                    Checkbox::new(SharedString::from(format!("level-{}", rung.label)))
                        .w(width - px(0.01) * count as f32)
                        .checked(on)
                        .disabled(on && levels.len() == 1)
                        .label(rung.label.clone())
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(match rung.elo {
                                    Some(elo) if elo != 0. => format!("~{elo}"),
                                    _ => "Full strength".into(),
                                }),
                        )
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            let ladder_ids = ladder_ids.clone();
                            crate::settings::change(cx, move |s| {
                                // Keep ladder order and at least one level.
                                let mut next = s.core.engine_levels.clone();
                                next.retain(|level| *level != id);
                                if checked {
                                    next.push(id);
                                }
                                if !next.is_empty() {
                                    s.core.engine_levels = ladder_ids
                                        .iter()
                                        .copied()
                                        .filter(|level| next.contains(level))
                                        .collect();
                                }
                            });
                        })
                }));
        let defaults = self.default_levels.clone();
        let reset = h_flex().child(
            Button::new("levels-default")
                .ghost()
                .icon(icon("rotate-ccw"))
                .label("Reset to default")
                .disabled(levels == self.default_levels)
                .on_click(move |_, _, cx| {
                    let defaults = defaults.clone();
                    crate::settings::change(cx, move |s| s.core.engine_levels = defaults);
                }),
        );
        let mut rows = Rows::new(self.narrow())
            .switch(
                "legal-moves",
                "Show possible moves",
                None,
                frontend.show_legal_moves,
                true,
                |s, on| s.frontend.show_legal_moves = on,
                palette,
            )
            .switch(
                "premove",
                "Premoves",
                Some("Plays your queued move automatically when legal."),
                frontend.premove,
                true,
                |s, on| s.frontend.premove = on,
                palette,
            )
            .row(
                "Pawn promotion",
                None,
                choice(
                    "promotion",
                    &[
                        (PromotionMode::Ask, "Ask me", None),
                        (PromotionMode::Queen, "Always queen", None),
                        (PromotionMode::Premove, "Queen when premoving", None),
                    ],
                    frontend.promotion,
                    |s, value| s.frontend.promotion = value,
                    palette,
                ),
                palette,
            )
            .stacked(
                "Computer levels",
                Some("Approximate engine ratings."),
                v_flex().gap(px(10.)).child(checkboxes).child(reset),
                palette,
            );
        let switches: [SwitchRow; 4] = [
            (
                "opening-names",
                "Show opening names",
                None,
                frontend.show_opening_name,
                |s, on| s.frontend.show_opening_name = on,
            ),
            (
                "blindfold",
                "Blindfold",
                Some("Hide pieces while playing."),
                frontend.blindfold,
                |s, on| s.frontend.blindfold = on,
            ),
            (
                "zen-mode",
                "Zen mode",
                Some("Board and game controls only · Z to toggle."),
                frontend.zen_mode,
                |s, on| s.frontend.zen_mode = on,
            ),
            (
                "puzzle-next",
                "Next puzzle after solving",
                None,
                frontend.puzzle_auto_next,
                |s, on| s.frontend.puzzle_auto_next = on,
            ),
        ];
        for (id, title, hint, on, set) in switches {
            rows = rows.switch(id, title, hint, on, true, set, palette);
        }
        rows.card(palette)
    }

    fn gestures(&self, settings: &Settings, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        Rows::new(self.narrow())
            .switch(
                "swipe-navigation",
                "Two-finger swipe",
                Some("Go back and forward with a horizontal swipe."),
                frontend.swipe_navigation,
                true,
                |s, on| s.frontend.swipe_navigation = on,
                palette,
            )
            .switch(
                "swipe-indicator",
                "Swipe feedback",
                Some("Show an edge hint that follows the swipe."),
                frontend.swipe_indicator,
                frontend.swipe_navigation,
                |s, on| s.frontend.swipe_indicator = on,
                palette,
            )
            .card(palette)
    }

    fn online(&self, settings: &Settings, palette: &Palette) -> Div {
        let core = &settings.core;
        Rows::new(self.narrow())
            .switch(
                "receive-challenges",
                "Receive challenges",
                None,
                core.receive_challenges,
                true,
                |s, on| s.core.receive_challenges = on,
                palette,
            )
            .switch(
                "online-chat",
                "Game chat",
                None,
                core.online_chat,
                true,
                |s, on| s.core.online_chat = on,
                palette,
            )
            .row(
                "Check correspondence games",
                Some("Only while KChess is open.".into()),
                Select::new(&self.poll).accessibility_label("Check correspondence games"),
                palette,
            )
            .card(palette)
    }

    fn analysis(&self, settings: &Settings, palette: &Palette) -> Div {
        Rows::new(self.narrow())
            .switch(
                "cloud-eval",
                "Lichess cloud evaluation",
                Some("Sends analyzed positions to Lichess. Paused during your online games."),
                settings.core.cloud_eval,
                true,
                |s, on| s.core.cloud_eval = on,
                palette,
            )
            .card(palette)
    }

    fn sound(&self, settings: &Settings, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        Rows::new(self.narrow())
            .switch(
                "sound",
                "Game sounds",
                None,
                frontend.sound_enabled,
                true,
                |s, on| s.frontend.sound_enabled = on,
                palette,
            )
            .row(
                "Volume",
                Some(format!("{}%", (frontend.sound_volume * 100.).round()).into()),
                Slider::new(&self.volume)
                    .disabled(!frontend.sound_enabled)
                    .accessibility_label("Volume"),
                palette,
            )
            .card(palette)
    }

    /// Delayed so there is time to switch to another app: macOS shows no banners for the app in
    /// front (`sendTest`).
    fn send_test(&mut self, cx: &mut Context<Self>) {
        const DELAY: u64 = 5;
        self.test_pending = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            for left in (1..=DELAY).rev() {
                let _ = this.update(cx, |this, cx| {
                    this.test_note = Some(format!("Switch to another app now… sending in {left}s").into());
                    cx.notify();
                });
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
            }
            let _ = this.update(cx, |this, cx| {
                let outcome = crate::notifications::desktop(
                    "test",
                    "KChess notifications",
                    "This is how game alerts will look. Click it to return to KChess.",
                    cx,
                );
                use crate::notifications::Outcome;
                this.test_note = Some(
                    match outcome {
                        Outcome::InApp => "Shown inside KChess. Switch to another app before it sends to try the system notification.",
                        Outcome::System => "Sent. If nothing appeared, check KChess in your system notification settings and that Focus is off.",
                        Outcome::Disabled => "Notifications are turned off.",
                        Outcome::CategoryOff => "That kind of notification is turned off.",
                        Outcome::WindowState | Outcome::Redundant => "Alerts are off for the current window state.",
                    }
                    .into(),
                );
                this.test_pending = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn notifications(&self, settings: &Settings, palette: &Palette, cx: &mut Context<Self>) -> Div {
        let frontend = &settings.frontend;
        let enabled = frontend.notifications_enabled;
        let mut rows = Rows::new(self.narrow()).switch(
            "notifications",
            "Desktop notifications",
            None,
            enabled,
            true,
            |s, on| s.frontend.notifications_enabled = on,
            palette,
        );
        let switches: [SwitchRow; 8] = [
            (
                "notify-active",
                "While I'm using KChess",
                None,
                frontend.notify_active,
                |s, on| s.frontend.notify_active = on,
            ),
            (
                "notify-background",
                "While KChess is in the background",
                None,
                frontend.notify_background,
                |s, on| s.frontend.notify_background = on,
            ),
            (
                "notify-opponent",
                "Opponent moves",
                None,
                frontend.notify_opponent_move,
                |s, on| s.frontend.notify_opponent_move = on,
            ),
            (
                "notify-low-time",
                "Low on time",
                None,
                frontend.notify_low_time,
                |s, on| s.frontend.notify_low_time = on,
            ),
            (
                "notify-games",
                "Game start and result",
                None,
                frontend.notify_game_events,
                |s, on| s.frontend.notify_game_events = on,
            ),
            (
                "notify-computer",
                "Computer moves",
                None,
                frontend.notify_computer_move,
                |s, on| s.frontend.notify_computer_move = on,
            ),
            (
                "notify-challenges",
                "Challenges",
                None,
                frontend.notify_challenges,
                |s, on| s.frontend.notify_challenges = on,
            ),
            (
                "notify-sound",
                "System sound",
                Some("For system notifications."),
                frontend.notify_sound,
                |s, on| s.frontend.notify_sound = on,
            ),
        ];
        for (id, title, hint, on, set) in switches {
            rows = rows.switch(id, title, hint, on, enabled, set, palette);
        }
        rows.row(
            "Try it",
            Some(
                self.test_note
                    .clone()
                    .unwrap_or_else(|| "Sends a sample in 5 seconds.".into()),
            ),
            h_flex()
                .gap(px(8.))
                .child(
                    Button::new("notify-system")
                        .ghost()
                        .icon(icon("settings-2"))
                        .label("System settings")
                        .on_click(cx.listener(|this, _, _, cx| {
                            // Fixed addresses only.
                            let url = if cfg!(target_os = "macos") {
                                Some("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
                            } else if cfg!(target_os = "windows") {
                                Some("ms-settings:notifications")
                            } else {
                                None
                            };
                            match url {
                                Some(url) => cx.open_url(url),
                                None => {
                                    this.test_note = Some(
                                        "Open your system notification settings and allow KChess there.".into(),
                                    );
                                    cx.notify();
                                }
                            }
                        })),
                )
                .child(
                    Button::new("notify-test")
                        .outline()
                        .icon(icon("bell-ring"))
                        .label("Send test")
                        .disabled(self.test_pending)
                        .on_click(cx.listener(|this, _, _, cx| this.send_test(cx))),
                ),
            palette,
        )
        .card(palette)
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let handle = cx.global::<SettingsHandle>().0.clone();
        let state = handle.read(cx);
        let settings = state.settings.clone();
        let body = match self.section {
            Section::Appearance => self.appearance(&settings, window, &palette),
            Section::Themes => self.themes(
                &settings,
                &state.custom_themes,
                &state.themes_dir,
                &state.theme_problems,
                window,
                &palette,
            ),
            Section::Gameplay => self.gameplay(&settings, window, &palette),
            Section::Gestures => self.gestures(&settings, &palette),
            Section::Online => self.online(&settings, &palette),
            Section::Analysis => self.analysis(&settings, &palette),
            Section::Sound => self.sound(&settings, &palette),
            Section::Notifications => self.notifications(&settings, &palette, cx),
            Section::Engine => self.engine(&settings, cx, &palette),
            Section::Offline => self.offline(cx, &palette),
            Section::Updates => self.updates(&settings, cx, &palette),
            Section::Accounts => self.accounts(cx, &palette),
            Section::Voice => self.voice(&settings, cx, &palette),
            Section::Data => self.data(cx, &palette),
        };
        self.sync_zoom(window, cx);
        div()
            .id("settings-page")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_y_scroll()
            .child(self.area.probe(cx))
            .child(
                v_flex()
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    // Named, not `super::*`: that would bring in GPUI's own `test` attribute.
    use super::{OPTION_GAP, OPTION_MIN, columns, theme_list};
    use gpui_kit::px;
    use kchess_contracts::types::AppTheme;
    use serde_json::json;

    #[test]
    fn pickers_fill_the_width_in_whole_columns() {
        let (count, item) = columns(px(900.), OPTION_MIN, OPTION_GAP);
        assert_eq!(count, 9);
        assert!((item * count as f32 + OPTION_GAP * 8. - px(900.)).abs() < px(0.01));
        assert_eq!(columns(px(50.), OPTION_MIN, OPTION_GAP).0, 1);
    }

    #[test]
    fn the_theme_list_puts_the_default_first_and_custom_themes_over_presets() {
        let presets = theme_list(&[]);
        assert_eq!(presets[0].id, "kchess");
        let id = presets[1].id.clone();
        let nord: AppTheme = serde_json::from_value(json!({ "id": id, "name": "Mine" })).unwrap();
        let themes = theme_list(&[nord]);
        assert_eq!(themes.len(), presets.len());
        assert_eq!(themes[1].name, "Mine");
        assert_eq!(themes[1].custom, Some(true));
    }
}
