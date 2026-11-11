//! Settings: one category at a time, picked in the sidebar while Settings is
//! open. Every change saves itself (`settings.rs`).

mod accounts;
mod ai;
mod data;
mod engine;
mod offline;
mod toggles;
pub(crate) use toggles::{Quick, TOGGLES, Toggle};
pub mod updates;
mod voice;
mod voice_history;

use kchess_domain::training::engine_levels;
use std::path::Path;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_contracts::types::{
    AppTheme, Appearance, CoordinateMode, EngineLevel, EngineStatus, PieceAnimation, PromotionMode,
    Settings,
};
use shakmaty::{Color, Piece, Role};

use crate::assets::icon;
use crate::board::looks;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, columns};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Themes,
    Gameplay,
    Gestures,
    Online,
    Analysis,
    Ai,
    Sound,
    Notifications,
    Voice,
    Engine,
    Accounts,
    Offline,
    Data,
    Updates,
}

/// The groups of categories the sidebar lists while Settings is open, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionGroup {
    Interface,
    Play,
    Analysis,
    Lichess,
    System,
}

impl SectionGroup {
    pub const ALL: [SectionGroup; 5] = [
        SectionGroup::Interface,
        SectionGroup::Play,
        SectionGroup::Analysis,
        SectionGroup::Lichess,
        SectionGroup::System,
    ];

    pub fn heading(self) -> &'static str {
        match self {
            SectionGroup::Interface => "Interface",
            SectionGroup::Play => "Play",
            SectionGroup::Analysis => "Analysis",
            SectionGroup::Lichess => "Lichess",
            SectionGroup::System => "System",
        }
    }

    /// Its categories, in the sidebar's order.
    pub fn sections(self) -> impl Iterator<Item = Section> {
        Section::ALL
            .into_iter()
            .filter(move |section| section.group() == self)
    }
}

impl Section {
    /// Every category, in the sidebar's order.
    pub const ALL: [Section; 15] = [
        Section::Appearance,
        Section::Themes,
        Section::Sound,
        Section::Notifications,
        Section::Gameplay,
        Section::Gestures,
        Section::Voice,
        Section::Engine,
        Section::Analysis,
        Section::Ai,
        Section::Accounts,
        Section::Online,
        Section::Offline,
        Section::Data,
        Section::Updates,
    ];

    pub fn label(self) -> &'static str {
        self.describe().0
    }

    pub fn icon(self) -> Icon {
        icon(self.describe().1)
    }

    pub fn group(self) -> SectionGroup {
        self.describe().2
    }

    /// Its name, icon and group, said once.
    fn describe(self) -> (&'static str, &'static str, SectionGroup) {
        use SectionGroup as G;
        match self {
            Section::Appearance => ("Appearance", "palette", G::Interface),
            Section::Themes => ("Themes", "swatch-book", G::Interface),
            Section::Sound => ("Sound", "volume-2", G::Interface),
            Section::Notifications => ("Notifications", "bell", G::Interface),
            Section::Gameplay => ("Gameplay", "chess-knight", G::Play),
            Section::Gestures => ("Gestures", "hand", G::Play),
            Section::Voice => ("Voice input", "mic", G::Play),
            Section::Engine => ("Chess engine", "cpu", G::Analysis),
            Section::Analysis => ("Cloud evaluation", "cloud", G::Analysis),
            Section::Ai => ("AI", "lightbulb", G::Analysis),
            Section::Accounts => ("Accounts", "user-round", G::Lichess),
            Section::Online => ("Online play", "globe-2", G::Lichess),
            Section::Offline => ("Offline downloads", "download", G::System),
            Section::Data => ("Data & storage", "database", G::System),
            Section::Updates => ("Updates", "refresh-cw", G::System),
        }
    }
}

/// The light and dark choice, as Settings and Quick Pick offer it: its name and icon.
pub const APPEARANCES: [(Appearance, &str, &str); 3] = [
    (Appearance::System, "System", "monitor"),
    (Appearance::Light, "Light", "sun"),
    (Appearance::Dark, "Dark", "moon"),
];

/// Where the board's coordinates go, as Settings and Quick Pick offer it.
pub const COORDINATES: [(CoordinateMode, &str); 3] = [
    (CoordinateMode::Inside, "Inside"),
    (CoordinateMode::Outside, "Outside"),
    (CoordinateMode::None, "Hidden"),
];

/// How the interface size changes anywhere: "⌘ + and ⌘ − adjust it anywhere."
fn zoom_hint(cx: &App) -> String {
    let (Some(bigger), Some(smaller)) = (
        crate::menu::keys(&crate::menu::ZoomIn, cx),
        crate::menu::keys(&crate::menu::ZoomOut, cx),
    ) else {
        return String::new();
    };
    format!(
        "{bigger} and {smaller} adjust it anywhere{}",
        if cfg!(target_os = "macos") { "." } else { "" }
    )
}

/// What finds a category in ⌘K besides its name: every row title it shows (checked by a test, so
/// a new row is found too), what the custom rows hold, and other names for it.
pub fn section_keywords(section: Section) -> String {
    toggles::titles(section)
        .chain([other_keywords(section)])
        .collect::<Vec<_>>()
        .join(" ")
}

/// A category's search words besides its toggles' titles.
fn other_keywords(section: Section) -> &'static str {
    match section {
        Section::Appearance => {
            "Theme Interface size zoom Board coordinates Piece animation Piece notation figurine \
             symbols letters Piece set Board theme colors"
        }
        Section::Themes => {
            "Appearance Light mode theme Dark mode theme Your own themes app theme colors custom"
        }
        Section::Sound => "Volume mute audio",
        Section::Notifications => "Try it alerts",
        Section::Gameplay => {
            "legal dots Pawn promotion \
             auto-queen Computer levels auto next"
        }
        Section::Gestures => "trackpad back forward",
        Section::Voice => "Microphone access speech",
        Section::Engine => {
            "Status Review my games automatically Game review Stockfish \
             executable download update"
        }
        Section::Analysis => "analysis",
        Section::Ai => "ChatGPT model effort usage coach explain",
        Section::Accounts => {
            "my accounts lichess sign in log in connect disconnect sync tracked Use for online play"
        }
        Section::Online => "Check correspondence games",
        Section::Offline => "Downloaded puzzles voice model Saved games and studies engine",
        Section::Data => {
            "Diagnostics Export diagnostics Clear local scores Reset counters Database file storage"
        }
        Section::Updates => {
            "version about upgrade Check for updates What's new release notes \
             Website Source code GitHub Report a problem issues bug"
        }
    }
}

/// Minutes between correspondence checks.
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

/// The theme pickers' and piece pickers' columns: at least 92px wide, 8px apart.
const OPTION_MIN: Pixels = px(92.);
const APP_THEME_MIN: Pixels = px(112.);
const OPTION_GAP: Pixels = px(8.);
/// Below this page width each row puts its control under its title.
const NARROW: Pixels = px(640.);

/// A rung of the engine ladder, as Settings lists it.
struct Rung {
    id: EngineLevel,
    label: String,
    elo: Option<f64>,
}

/// A ladder level's id as Settings stores it.
fn level_id(id: &str) -> Result<EngineLevel, String> {
    serde_json::from_value(serde_json::Value::String(id.into()))
        .map_err(|cause| format!("engine level {id}: {cause}"))
}

pub struct SettingsPage {
    section: Section,
    area: Measured,
    volume: Entity<SliderState>,
    poll: Entity<SelectState<Vec<SharedString>>>,
    /// Interface size: Auto (its percent here) and the steps; refilled when either changes.
    zoom: Entity<SelectState<Vec<SharedString>>>,
    zoom_shown: Option<(crate::interface_zoom::Setting, u16)>,
    ai_model: Entity<SelectState<Vec<SharedString>>>,
    ai_effort: Entity<SelectState<Vec<SharedString>>>,
    /// The models and choices the pickers show.
    ai_shown: Option<ai::Shown>,
    ladder: Vec<Rung>,
    default_levels: Vec<EngineLevel>,
    /// The engine the core finds, and whether it is being downloaded or deleted.
    engine: Option<EngineStatus>,
    engine_busy: bool,
    /// What the notification test says, and whether it is counting down.
    test_note: Option<SharedString>,
    test_pending: bool,
    /// The microphone test, and why it could not start.
    mic_test: Option<voice::MicTest>,
    mic_error: Option<SharedString>,
    /// What the system says about the microphone.
    mic: crate::microphone::Host,
    mic_asking: bool,
    /// Data & storage: what clearing the scores and exporting diagnostics said.
    runs_note: Option<SharedString>,
    diagnostics_note: Option<SharedString>,
    exporting: bool,
    voice_history: Option<Entity<voice_history::VoiceHistory>>,
    /// The offline speech model, and whether this page is downloading it.
    voice_model: Option<kchess_contracts::voice::Status>,
    voice_model_error: Option<SharedString>,
    voice_downloading: bool,
    /// My accounts: the Chess.com username to add.
    chesscom_input: Entity<gpui_kit::component::input::InputState>,
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
        let ai_model = cx.new(|cx| SelectState::new(Vec::<SharedString>::new(), None, window, cx));
        let ai_effort = cx.new(|cx| {
            SelectState::new(
                ai::EFFORTS
                    .iter()
                    .map(|(_, label)| SharedString::from(*label))
                    .collect(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let chesscom_input = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx)
                .placeholder("Chess.com username")
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &chesscom_input,
                window,
                |this, _, event: &gpui_kit::component::input::InputEvent, window, cx| match event {
                    gpui_kit::component::input::InputEvent::PressEnter { .. } => {
                        this.add_chesscom(window, cx)
                    }
                    gpui_kit::component::input::InputEvent::Change => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe(
                &ai_model,
                |this, _, _: &SelectEvent<Vec<SharedString>>, cx| {
                    this.save_ai(cx);
                },
            ),
            cx.subscribe(
                &ai_effort,
                |this, _, _: &SelectEvent<Vec<SharedString>>, cx| {
                    this.save_ai(cx);
                },
            ),
            cx.observe(&crate::ai::handle(cx), |_, _, cx| cx.notify()),
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
                // The slider's steps are f32: keep the hundredths the setting stores.
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
        let ladder = engine_levels::ladder()
            .iter()
            .map(|rung| {
                Ok(Rung {
                    id: level_id(&rung.id)?,
                    label: rung.label.clone(),
                    elo: rung.elo,
                })
            })
            .collect::<Result<_, String>>()
            .unwrap_or_else(|cause| {
                crate::log::write(Level::Warn, "settings", &cause);
                Vec::new()
            });
        let default_levels = engine_levels::default_levels()
            .iter()
            .map(|id| level_id(id))
            .collect::<Result<_, String>>()
            .unwrap_or_else(|cause| {
                crate::log::write(Level::Warn, "settings", &cause);
                Vec::new()
            });
        SettingsPage {
            section: Section::Appearance,
            area: Measured::default(),
            volume,
            poll,
            zoom,
            zoom_shown: None,
            ai_model,
            ai_effort,
            ai_shown: None,
            ladder,
            default_levels,
            engine: None,
            engine_busy: false,
            test_note: None,
            test_pending: false,
            mic_test: None,
            mic_error: None,
            mic: crate::microphone::host(cx),
            mic_asking: false,
            runs_note: None,
            diagnostics_note: None,
            exporting: false,
            voice_history: None,
            voice_model: None,
            voice_model_error: None,
            voice_downloading: false,
            chesscom_input,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// Show one category, from the top.
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
        if section == Section::Ai {
            crate::ai::handle(cx).update(cx, |ai, cx| ai.refresh(cx));
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

/// A row's title and its optional hint.
fn info(title: impl Into<SharedString>, hint: Option<SharedString>, palette: &Palette) -> Div {
    widgets::setting_info(title, hint, palette)
}

/// The category's rows, with dividers between them.
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

    /// A setting row: the title and its hint on the left, the control at the right edge.
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
        let row = if self.narrow {
            v_flex()
                .gap(px(8.))
                .child(info)
                .child(div().max_w_full().child(control))
                .into_any_element()
        } else {
            widgets::setting_row(info.min_w(px(200.)), control).into_any_element()
        };
        self.rows.push(row);
        self
    }

    /// A yes/no row (`toggles`): the switch sits at the start of the control column.
    fn switch(
        mut self,
        id: &'static str,
        settings: &Settings,
        enabled: bool,
        palette: &Palette,
    ) -> Rows {
        let toggle = toggles::toggle(id);
        let set = toggle.set;
        let control = h_flex().child(
            Switch::new(id)
                .large()
                .checked((toggle.on)(settings))
                .disabled(!enabled)
                .accessibility_label(toggle.title)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    crate::settings::change(cx, move |settings| set(settings, checked));
                }),
        );
        // A switch is small enough to stay beside its title in the narrowest window.
        self.rows.push(
            widgets::setting_row(
                info(toggle.title, toggle.hint.map(SharedString::from), palette).min_w_0(),
                control,
            )
            .into_any_element(),
        );
        self
    }

    /// A control that needs the full width goes under its title.
    fn stacked(
        mut self,
        title: &'static str,
        hint: Option<&'static str>,
        control: impl IntoElement,
        palette: &Palette,
    ) -> Rows {
        self.rows.push(
            v_flex()
                .gap(px(8.))
                .child(info(title, hint.map(SharedString::from), palette))
                .child(control)
                .into_any_element(),
        );
        self
    }

    fn card(self, palette: &Palette) -> Div {
        self.card_under(None, palette)
    }

    /// The rows in a card that opens with `header` (a title and its status).
    fn card_under(self, header: Option<AnyElement>, palette: &Palette) -> Div {
        // 4px above and below: the rows bring their own padding.
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

/// A segmented choice that changes one setting.
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
    widgets::setting_pills(
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

/// A picture and its name; the chosen one is ringed in the primary color.
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
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(name.clone())
        .aria_selected(chosen)
        .w(width)
        .items_center()
        .gap(px(4.))
        .py(px(6.))
        .px(px(4.))
        .rounded(radii::LG)
        .bg(palette.bg)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(chosen, |option| {
            // The border and its 1px outline.
            option
                .border_2()
                .border_color(palette.primary)
                .py(px(4.))
                .px(px(4.))
                .text_color(palette.highlighted)
        })
        .when(!chosen, |option| {
            option
                .border_1()
                .border_color(palette.border)
                .text_color(palette.text_muted)
                .press(
                    |option| {
                        option
                            .border_color(palette.border_accented)
                            .bg(palette.accented)
                            .text_color(palette.toned)
                    },
                    |option| option.bg(palette.pressed(palette.accented)),
                )
        })
        .on_click(move |_, _, cx| choose(cx))
        .child(picture)
        .child(name)
        .children(tag.map(|tag| {
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.primary)
                .child(tag)
        }))
}

/// Lay pickers out in rows of `count`, `OPTION_GAP` apart.
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
    // Each square takes the preview's outer corners: GPUI clips to rectangles, not rounded ones.
    let square = |piece: Piece, color: u32, first: bool| {
        crate::widgets::bar_segment(div(), (first, !first), radii::SM)
            .size(side)
            .bg(rgb(color))
            .child(
                img(SharedString::from(looks::piece_path(set, piece)))
                    .size(side)
                    .object_fit(ObjectFit::Cover),
            )
    };
    h_flex()
        .rounded(radii::SM)
        .overflow_hidden()
        .child(square(
            Piece {
                color: Color::White,
                role: Role::Knight,
            },
            0xf0d9b5,
            true,
        ))
        .child(square(
            Piece {
                color: Color::Black,
                role: Role::Queen,
            },
            0xb58863,
            false,
        ))
}

fn board_swatch(image: &str, width: Pixels) -> impl IntoElement {
    img(SharedString::from(format!("boards/{image}")))
        .size(picture_width(width))
        .rounded(radii::SM)
        .object_fit(ObjectFit::Cover)
}

/// The theme's background, a raised card, its text and primary colors. A scale
/// model of a page, so its corners are scaled down with it rather than taken from `radii`.
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
                .gap(px(4.))
                .p(px(4.))
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

/// The default look, the presets, then the user's own (replacing a preset's id).
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
    APPEARANCES.map(|(appearance, label, glyph)| (appearance, label, Some(icon(glyph))))
}

impl SettingsPage {
    fn appearance(&self, settings: &Settings, window: &Window, palette: &Palette, cx: &App) -> Div {
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
                Some(zoom_hint(cx).into()),
                widgets::setting_field(
                    Select::new(&self.zoom).accessibility_label("Interface size"),
                ),
                palette,
            )
            .row(
                "Board coordinates",
                None,
                choice(
                    "coordinates",
                    &COORDINATES.map(|(mode, label)| (mode, label, None)),
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
            .row(
                "Piece notation",
                None,
                choice(
                    "notation",
                    &[(false, "Letters", None), (true, "Symbols", None)],
                    frontend.figurine_notation,
                    |s, value| s.frontend.figurine_notation = value,
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
            .gap(px(8.))
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
        // As many columns as fit, each at least 192px, 12px by 24px apart.
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
            .switch("legal-moves", settings, true, palette)
            .switch("premove", settings, true, palette)
            .switch("confirm-move", settings, true, palette)
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
                Some("Approximate engine ratings"),
                v_flex().gap(px(8.)).child(checkboxes).child(reset),
                palette,
            );
        for id in ["opening-names", "blindfold", "zen-mode", "puzzle-next"] {
            rows = rows.switch(id, settings, true, palette);
        }
        rows.card(palette)
    }

    fn gestures(&self, settings: &Settings, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        Rows::new(self.narrow())
            .switch("swipe-navigation", settings, true, palette)
            .switch(
                "swipe-indicator",
                settings,
                frontend.swipe_navigation,
                palette,
            )
            .card(palette)
    }

    fn online(&self, settings: &Settings, palette: &Palette) -> Div {
        Rows::new(self.narrow())
            .switch("receive-challenges", settings, true, palette)
            .switch("online-chat", settings, true, palette)
            .row(
                "Check correspondence games",
                Some("Only while KChess is open".into()),
                widgets::setting_field(
                    Select::new(&self.poll).accessibility_label("Check correspondence games"),
                ),
                palette,
            )
            .card(palette)
    }

    fn analysis(&self, settings: &Settings, palette: &Palette) -> Div {
        Rows::new(self.narrow())
            .switch("cloud-eval", settings, true, palette)
            .card(palette)
    }

    fn sound(&self, settings: &Settings, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        Rows::new(self.narrow())
            .switch("sound", settings, true, palette)
            .row(
                "Volume",
                Some(format!("{}%", (frontend.sound_volume * 100.).round()).into()),
                widgets::setting_field(
                    Slider::new(&self.volume)
                        .disabled(!frontend.sound_enabled)
                        .accessibility_label("Volume"),
                ),
                palette,
            )
            .card(palette)
    }

    /// Delayed so there is time to switch to another app: macOS shows no banners for the app in
    /// front.
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
                        Outcome::Disabled => "Notifications are turned off",
                        Outcome::CategoryOff => "That kind of notification is turned off",
                        Outcome::WindowState | Outcome::Redundant => "Alerts are off for the current window state",
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
        let mut rows = Rows::new(self.narrow()).switch("notifications", settings, true, palette);
        for id in [
            "notify-active",
            "notify-background",
            "notify-opponent",
            "notify-low-time",
            "notify-games",
            "notify-computer",
            "notify-challenges",
            "notify-sound",
        ] {
            rows = rows.switch(id, settings, enabled, palette);
        }
        rows.row(
            "Try it",
            Some(
                self.test_note
                    .clone()
                    .unwrap_or_else(|| "Sends a sample in 5 seconds".into()),
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
                                        "Open your system notification settings and allow KChess there".into(),
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
            Section::Appearance => self.appearance(&settings, window, &palette, cx),
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
            Section::Ai => self.ai_section(cx, &palette),
            Section::Sound => self.sound(&settings, &palette),
            Section::Notifications => self.notifications(&settings, &palette, cx),
            Section::Engine => self.engine(&settings, cx, &palette),
            Section::Offline => self.offline(cx, &palette),
            Section::Updates => self.updates(&settings, cx, &palette),
            Section::Accounts => v_flex()
                .gap(px(16.))
                .child(self.accounts(cx, &palette))
                .child(self.chesscom_accounts(cx, &palette)),
            Section::Voice => self.voice(&settings, cx, &palette),
            Section::Data => self.data(cx, &palette),
        };
        self.sync_zoom(window, cx);
        self.sync_ai(window, cx);
        div()
            .id("settings-page")
            .test_support()
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

impl crate::pages::PageView for SettingsPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Settings,
        label: "Settings",
        short: "Settings",
        icon: "settings-2",
        group: None,
        keywords: "preferences options",
    };

    type Action = std::convert::Infallible;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        SettingsPage::new(window, cx)
    }

    /// The microphone test runs only while its section is shown.
    fn hidden(this: &Entity<Self>, _window: &mut Window, cx: &mut App) {
        this.update(cx, |settings, cx| settings.stop_mic_test(cx));
    }

    fn opened_by() -> Option<Box<dyn gpui_kit::Action>> {
        Some(Box::new(crate::menu::OpenSettings))
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
