//! Settings: one category at a time, picked in the sidebar while Settings is
//! open. Every change saves itself (`settings.rs`).

mod accounts;
mod ai;
mod appearance;
mod data;
pub(crate) mod defaults;
mod engine;
mod rows;
mod speech_model;
mod storage;
mod tablebases;
mod toggles;
pub(crate) use toggles::{Quick, TOGGLES, Toggle};
pub mod updates;
mod voice;
mod voice_history;

use rows::*;

use crate::pages::PageView;
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
    AppTheme, Appearance, CloseAction, CoordinateMode, EngineLevel, EngineStatus, PieceAnimation,
    ProfileArea, PromotionMode, Settings,
};
use shakmaty::{Color, Piece, Role};

use crate::assets::icon;
use crate::board::looks;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, columns};

// Its categories, as other pages open Settings at one (`kchess_nav::sections`).
pub use kchess_nav::sections::{Section, SectionGroup};

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

/// How fast pieces move, as Settings offers it.
const ANIMATIONS: [(PieceAnimation, &str); 4] = [
    (PieceAnimation::None, "Off"),
    (PieceAnimation::Fast, "Fast"),
    (PieceAnimation::Normal, "Normal"),
    (PieceAnimation::Slow, "Slow"),
];

/// What closing the window does.
const CLOSES: [(CloseAction, &str); 2] = [
    (CloseAction::Background, "Keep running"),
    (CloseAction::Quit, "Quit"),
];

/// How moves name their pieces.
const NOTATIONS: [(bool, &str); 2] = [(false, "Letters"), (true, "Symbols")];

/// What a promoting pawn becomes.
const PROMOTIONS: [(PromotionMode, &str); 3] = [
    (PromotionMode::Ask, "Ask me"),
    (PromotionMode::Queen, "Always queen"),
    (PromotionMode::Premove, "Queen when premoving"),
];

/// A segmented choice's items, from its values and names.
fn items<T: Copy, const N: usize>(
    named: &[(T, &'static str); N],
) -> [(T, &'static str, Option<Icon>); N] {
    named.map(|(value, name)| (value, name, None))
}

/// The name of `value` among `named` (a choice's default, as its reset names it).
fn name_of<T: PartialEq>(named: &[(T, &'static str)], value: T) -> SharedString {
    named
        .iter()
        .find(|(of, _)| *of == value)
        .map(|(_, name)| SharedString::new_static(name))
        .unwrap_or_default()
}

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

/// The titles of a category's rows, as they are drawn (a test checks every drawn title is here,
/// so a new row is found too): ⌘K finds the category by each, and opens it at that row.
pub fn section_rows(section: Section) -> Vec<&'static str> {
    toggles::titles(section)
        .chain(other_rows(section).iter().copied())
        .collect()
}

/// What finds a category in ⌘K besides its name: its rows' titles, what the custom rows hold,
/// and other names for it.
pub fn section_keywords(section: Section) -> String {
    section_rows(section)
        .into_iter()
        .chain([other_words(section)])
        .collect::<Vec<_>>()
        .join(" ")
}

/// The titles of a category's rows that are not toggles.
fn other_rows(section: Section) -> &'static [&'static str] {
    match section {
        Section::Appearance => &[
            "Theme",
            "Interface size",
            "Board coordinates",
            "Piece animation",
            "Piece notation",
            "Piece set",
            "Board theme",
        ],
        Section::Themes => &[
            "Appearance",
            "Light mode theme",
            "Dark mode theme",
            "Your own themes",
        ],
        Section::Sound => &["Volume"],
        Section::Notifications => &["Try it", "When you close the window"],
        Section::Gameplay => &["Pawn promotion", "Computer levels"],
        Section::Voice => &["Microphone access", "Test your microphone"],
        Section::Engine => &["Status", "Game review", "Review my games automatically"],
        Section::Ai => &["ChatGPT"],
        Section::Online => &["Check correspondence games"],
        Section::Storage => &["Local scores", "Diagnostics"],
        Section::Gestures | Section::Analysis | Section::Accounts | Section::Updates => &[],
    }
}

/// A category's other search words: what its custom rows hold, and other names for it.
fn other_words(section: Section) -> &'static str {
    match section {
        Section::Appearance => "zoom figurine symbols letters colors",
        Section::Themes => "app theme colors custom",
        Section::Sound => "mute audio",
        Section::Notifications => "alerts background tray menu bar startup login minimize quit",
        Section::Gameplay => "legal dots auto-queen auto next",
        Section::Gestures => "trackpad back forward",
        Section::Voice => "speech",
        Section::Engine => "Stockfish executable download update",
        Section::Analysis => "analysis",
        Section::Ai => "model effort usage coach explain",
        Section::Accounts => {
            "my accounts lichess sign in log in connect disconnect sync tracked Use for online play"
        }
        Section::Online => "",
        Section::Storage => {
            "Offline downloads Data disk space free Downloads Game databases Puzzles voice speech \
             model Stockfish engine endgame tablebases syzygy Accounts downloaded Export diagnostics Clear local scores \
             Reset counters"
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
    /// The page's scroll, which a revealed row moves.
    scroll: ScrollHandle,
    /// The row ⌘K opened the category at, lit for a moment.
    reveal: Option<Reveal>,
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
    /// The engine the core finds, and whether it is being downloaded or deleted.
    engine: Option<EngineStatus>,
    engine_busy: bool,
    /// What the notification test says, and whether it is counting down.
    test_note: Option<SharedString>,
    test_pending: bool,
    /// The microphone test, and why it could not start.
    mic_test: Option<voice::MicTest>,
    /// The microphone the test is opening; dropped, its answer goes nowhere.
    mic_opening: Option<Task<()>>,
    mic_error: Option<SharedString>,
    /// What the system says about the microphone.
    mic: crate::microphone::Host,
    mic_asking: bool,
    /// Storage: the bytes free on the profile's disk, when the system says.
    free_space: Option<u64>,
    /// Storage: every file in the profile, and the installed app with its service's copy.
    profile_bytes: Option<u64>,
    app_bytes: Option<u64>,
    /// Storage: what clearing the scores and exporting diagnostics said.
    runs_note: Option<SharedString>,
    diagnostics_note: Option<SharedString>,
    exporting: bool,
    voice_history: Option<Entity<voice_history::VoiceHistory>>,
    /// The offline speech model, and whether this page is downloading it.
    voice_model: Option<kchess_contracts::voice::Status>,
    voice_model_error: Option<SharedString>,
    voice_downloading: bool,
    /// The endgame tablebases, a download's progress and what went wrong.
    tablebases: Option<kchess_contracts::types::TablebaseStatus>,
    tablebase_progress: Option<kchess_contracts::types::TablebaseProgress>,
    tablebase_error: Option<SharedString>,
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
        let page = cx.entity().downgrade();
        let subscriptions = vec![
            // Whether the tray icon shows decides what closing the window can do.
            cx.observe_global::<crate::background::Background>(|_, cx| cx.notify()),
            crate::core::profile_changes(cx, &[ProfileArea::Engine], {
                let page = page.clone();
                move |cx| {
                    page.update(cx, |page, cx| page.engine_changed(cx)).ok();
                }
            }),
            crate::core::profile_changes(cx, &[ProfileArea::Tablebases], {
                let page = page.clone();
                move |cx| {
                    page.update(cx, |page, cx| {
                        if page.section == Section::Storage {
                            page.refresh_tablebases(cx);
                        }
                    })
                    .ok();
                }
            }),
            // A tablebase download's progress, whichever window started it.
            cx.subscribe(
                &cx.global::<crate::core::AppCore>().events.clone(),
                |this, _, event: &crate::core::ClientEvent, cx| {
                    if let crate::core::ClientEvent::Event(
                        crate::core::CoreEvent::TablebaseProgress(progress),
                    ) = event
                    {
                        this.tablebase_progress(progress, cx);
                    }
                },
            ),
            crate::core::profile_changes(cx, &[ProfileArea::VoiceHistory], move |cx| {
                page.update(cx, |page, cx| page.voice_history_changed(cx))
                    .ok();
            }),
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
                    let before = crate::interface_zoom::setting(cx);
                    if before == setting {
                        return;
                    }
                    choose(setting, window, cx);
                    this.zoom_shown = None;
                    cx.notify();
                    let page = cx.entity().downgrade();
                    crate::setting_notice::offer(
                        "Interface size changed",
                        move |window, cx| {
                            choose(before, window, cx);
                            // The list shows the size again (`sync_zoom`).
                            page.update(cx, |this, cx| {
                                this.zoom_shown = None;
                                cx.notify();
                            })
                            .ok();
                        },
                        window,
                        cx,
                    );
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
            // A download or delete that ended changes what Storage measures.
            cx.observe(
                &cx.global::<crate::puzzle_db::PuzzleDbHandle>().0.clone(),
                |this, db, cx| {
                    if this.section == Section::Storage && !db.read(cx).busy() {
                        this.refresh_free_space(cx);
                    }
                    cx.notify()
                },
            ),
            // Storage lists the game databases and their size.
            cx.observe(
                &cx.global::<crate::game_db::GameDbHandle>().0.clone(),
                |this, db, cx| {
                    if this.section == Section::Storage && !db.read(cx).busy() {
                        this.refresh_free_space(cx);
                    }
                    cx.notify()
                },
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
        SettingsPage {
            section: Section::Appearance,
            area: Measured::default(),
            scroll: ScrollHandle::new(),
            reveal: None,
            volume,
            poll,
            zoom,
            zoom_shown: None,
            ai_model,
            ai_effort,
            ai_shown: None,
            ladder,
            engine: None,
            engine_busy: false,
            test_note: None,
            test_pending: false,
            mic_test: None,
            mic_opening: None,
            mic_error: None,
            mic: crate::microphone::host(cx),
            mic_asking: false,
            free_space: None,
            profile_bytes: None,
            app_bytes: None,
            runs_note: None,
            diagnostics_note: None,
            exporting: false,
            voice_history: None,
            voice_model: None,
            voice_model_error: None,
            voice_downloading: false,
            tablebases: None,
            tablebase_progress: None,
            tablebase_error: None,
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
        self.reveal = None;
        self.scroll.set_offset(Point::default());
        if section == Section::Voice {
            self.refresh_microphone(cx);
            match &self.voice_history {
                Some(history) => history.update(cx, |history, cx| history.load(cx)),
                None => {
                    self.voice_history = Some(cx.new(voice_history::VoiceHistory::new));
                }
            }
        }
        if matches!(section, Section::Engine | Section::Storage) {
            self.refresh_engine(cx);
        }
        if matches!(section, Section::Accounts | Section::Storage) {
            crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        }
        if section == Section::Ai {
            crate::ai::handle(cx).update(cx, |ai, cx| ai.refresh(cx));
        }
        if section == Section::Storage {
            self.voice_model_error = None;
            self.refresh_voice_model(cx);
            self.tablebase_error = None;
            self.refresh_tablebases(cx);
            self.refresh_free_space(cx);
            let db = cx.global::<crate::puzzle_db::PuzzleDbHandle>().0.clone();
            db.update(cx, |db, cx| db.refresh(cx));
        }
        cx.notify();
    }

    /// Another window installed or deleted the engine: the engine shown is read again.
    fn engine_changed(&mut self, cx: &mut Context<Self>) {
        if matches!(self.section, Section::Engine | Section::Storage) {
            self.refresh_engine(cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn engine_status(&self) -> Option<&EngineStatus> {
        self.engine.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn forget_engine_status(&mut self) {
        self.engine = None;
    }

    /// Another window saved or cleared voice practice: the history shown is read again.
    fn voice_history_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(history) = &self.voice_history {
            history.update(cx, |history, cx| history.load(cx));
        }
    }

    /// The Interface size select shows the setting, and what Auto means on this display.
    fn sync_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::interface_zoom::{PERCENTS, Setting, auto_for};
        let shown = (crate::interface_zoom::setting(cx), auto_for(window, cx));
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

    /// Show one category scrolled to the row titled `title` (as `section_rows` names it), which
    /// is lit for a moment so the eye finds it: ⌘K found the category by that row.
    pub fn reveal(&mut self, section: Section, title: &'static str, cx: &mut Context<Self>) {
        self.show(section, cx);
        let fade = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_LIT).await;
            this.update(cx, |this, cx| {
                this.reveal = None;
                cx.notify();
            })
            .ok();
        });
        self.reveal = Some(Reveal {
            title,
            pin: std::rc::Rc::new(std::cell::Cell::new(Pin::Start)),
            scroll: self.scroll.clone(),
            _fade: fade,
        });
        cx.notify();
    }

    /// The row the page is revealing, if any.
    #[cfg(test)]
    pub fn revealed(&self) -> Option<&'static str> {
        self.reveal.as_ref().map(|reveal| reveal.title)
    }

    /// A category's rows, the row being revealed among them marked.
    fn rows(&self) -> Rows {
        Rows {
            narrow: self.narrow(),
            reveal: self.reveal.as_ref().map(Reveal::mark),
            rows: Vec::new(),
            shown: std::cell::OnceCell::new(),
        }
    }
}

impl SettingsPage {
    fn gameplay(&self, settings: &Settings, window: &Window, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        let levels = settings.core.engine_levels.clone();
        let ladder_ids: Vec<EngineLevel> = self.ladder.iter().map(|rung| rung.id).collect();
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
        let mut rows = self
            .rows()
            .switch("legal-moves", settings, true, palette)
            .switch("premove", settings, true, palette)
            .switch("confirm-move", settings, true, palette)
            .row(
                "Pawn promotion",
                None,
                choice(
                    "promotion",
                    &items(&PROMOTIONS),
                    frontend.promotion,
                    |s, value| s.frontend.promotion = value,
                    palette,
                ),
                Some(defaults::Reset::new("promotion", settings, |d| {
                    name_of(&PROMOTIONS, d.frontend.promotion)
                })),
                palette,
            )
            .stacked(
                "Computer levels",
                Some("Approximate engine ratings"),
                checkboxes,
                Some(defaults::Reset::new("engineLevels", settings, |d| {
                    format!("{} levels", d.core.engine_levels.len()).into()
                })),
                palette,
            );
        for id in ["opening-names", "blindfold", "zen-mode", "puzzle-next"] {
            rows = rows.switch(id, settings, true, palette);
        }
        rows.card(palette)
    }

    fn gestures(&self, settings: &Settings, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        self.rows()
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
        self.rows()
            .switch("receive-challenges", settings, true, palette)
            .switch("online-chat", settings, true, palette)
            .row(
                "Check correspondence games",
                Some("Only while KChess is open".into()),
                widgets::setting_field(
                    Select::new(&self.poll).accessibility_label("Check correspondence games"),
                ),
                Some(defaults::Reset::new("correspondencePoll", settings, |d| {
                    name_of(&POLL, d.core.correspondence_poll)
                })),
                palette,
            )
            .card(palette)
    }

    fn analysis(&self, settings: &Settings, palette: &Palette) -> Div {
        self.rows()
            .switch("cloud-eval", settings, true, palette)
            .card(palette)
    }

    fn sound(&self, settings: &Settings, palette: &Palette) -> Div {
        let frontend = &settings.frontend;
        self.rows()
            .switch("sound", settings, true, palette)
            .row(
                "Volume",
                Some(format!("{}%", (frontend.sound_volume * 100.).round()).into()),
                widgets::setting_field(
                    Slider::new(&self.volume)
                        .disabled(!frontend.sound_enabled)
                        .accessibility_label("Volume"),
                ),
                Some(defaults::Reset::new("soundVolume", settings, |d| {
                    format!("{}%", (d.frontend.sound_volume * 100.).round()).into()
                })),
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
                this.update(cx, |this, cx| {
                    this.test_note = Some(format!("Switch to another app now… sending in {left}s").into());
                    cx.notify();
                })
.ok();
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
            }
            this.update(cx, |this, cx| {
                let outcome = crate::notifications::desktop(
                    kchess_contracts::types::NotificationKind::Test,
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
            })
.ok();
        })
        .detach();
    }

    /// Running in the background: what closing the window does, the menu bar or tray icon, and
    /// opening at login.
    fn background(&self, settings: &Settings, palette: &Palette, cx: &App) -> Div {
        let frontend = &settings.frontend;
        let shown = crate::background::chosen(frontend.close_action);
        // Where keeping running is chosen but cannot be: nothing would show KChess runs.
        let hint = (shown == CloseAction::Background
            && !crate::background::keeps_running_with(
                frontend.close_action,
                crate::background::tray_shown(cx),
            ))
        .then(|| {
            if crate::background::tray_unavailable(cx).is_some() {
                SharedString::new_static("No system tray was found, so closing quits")
            } else {
                SharedString::new_static("Quits while the tray icon is off")
            }
        });
        let mut rows = self
            .rows()
            .row(
                "When you close the window",
                hint,
                choice(
                    "close-action",
                    &items(&CLOSES),
                    shown,
                    |s, value| {
                        // The platform's own choice is kept as the default, so it follows the
                        // platform and shows no reset.
                        s.frontend.close_action =
                            if crate::background::chosen(CloseAction::Default) == value {
                                CloseAction::Default
                            } else {
                                value
                            };
                    },
                    palette,
                ),
                Some(defaults::Reset::new("closeAction", settings, |d| {
                    name_of(&CLOSES, crate::background::chosen(d.frontend.close_action))
                })),
                palette,
            )
            .switch("show-tray", settings, true, palette);
        if crate::background::opens_at_login_here() {
            rows = rows.switch("open-at-login", settings, true, palette);
        }
        rows.card(palette)
    }

    fn notifications(&self, settings: &Settings, palette: &Palette, cx: &mut Context<Self>) -> Div {
        let frontend = &settings.frontend;
        let enabled = frontend.notifications_enabled;
        let mut rows = self.rows().switch("notifications", settings, true, palette);
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
            None,
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
            Section::Notifications => v_flex()
                .gap(px(16.))
                .child(self.notifications(&settings, &palette, cx))
                .child(self.background(&settings, &palette, cx)),
            Section::Engine => self.engine(&settings, cx, &palette),
            Section::Updates => self.updates(&settings, cx, &palette),
            Section::Accounts => v_flex()
                .gap(px(16.))
                .child(self.accounts(cx, &palette))
                .child(self.chesscom_accounts(cx, &palette)),
            Section::Voice => self.voice(&settings, cx, &palette),
            Section::Storage => self.storage(cx, &palette),
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
            .track_scroll(&self.scroll)
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

impl PageView for SettingsPage {
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
    use super::appearance::theme_list;
    use super::{OPTION_GAP, OPTION_MIN, columns};
    use gpui_kit::px;
    use kchess_contracts::types::AppTheme;
    use serde_json::json;

    #[test]
    fn a_revealed_row_is_kept_near_the_top_until_the_person_scrolls() {
        use super::{Pin, REVEAL_ROOM, pin_scroll};
        let (view, max) = (px(100.), px(500.));
        // A row 400 px down a page not yet scrolled: up 300, less the room above it.
        let first = pin_scroll(Pin::Start, px(0.), view, px(400.), max);
        assert_eq!(first, Some(px(-300.) + REVEAL_ROOM));
        // Pictures above it loaded and pushed it 50 px down: the page follows it.
        let set = first.unwrap();
        let moved = pin_scroll(Pin::At(set), set, view, view + REVEAL_ROOM + px(50.), max);
        assert_eq!(moved, Some(set - px(50.)));
        // Where it is: nothing to move.
        assert_eq!(
            pin_scroll(Pin::At(set), set, view, view + REVEAL_ROOM, max),
            Some(set)
        );
        // Never past the end of the page, nor above its top.
        assert_eq!(
            pin_scroll(Pin::Start, px(0.), view, px(2000.), max),
            Some(-max)
        );
        assert_eq!(
            pin_scroll(Pin::Start, px(0.), view, px(50.), max),
            Some(px(0.))
        );
        // The person scrolled: the page lets go, and stays let go.
        assert_eq!(
            pin_scroll(Pin::At(set), set - px(10.), view, px(400.), max),
            None
        );
        assert_eq!(pin_scroll(Pin::Left, px(0.), view, px(400.), max), None);
    }

    /// Regression: an option's frame (8px) sat 6px from a picture rounded 8px too, so the two
    /// corners did not follow each other. The picture is the option's corner less its inset.
    #[test]
    fn an_options_picture_is_concentric_with_its_frame() {
        assert_eq!(
            crate::theme::radii::BOX - super::rows::OPTION_INSET,
            crate::theme::radii::SMALL
        );
        assert_eq!(
            super::appearance::picture_width(px(100.)) + super::rows::OPTION_INSET * 2.,
            px(100.)
        );
    }

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
