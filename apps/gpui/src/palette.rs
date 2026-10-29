//! The top bar's search box and command palette:
//! ⌘K searches pages and commands, `>` runs commands, `@` finds players, `#` changes settings,
//! `?` explains the prefixes. Results show under the box; ↑/↓ choose, Return runs, Esc closes.

use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{Appearance, CoordinateMode, Settings};
use serde_json::json;

use crate::assets::icon;
use crate::online::Phase;
use crate::pages::Page;
use crate::pages::settings::Section;
use crate::quick_pick::{self, Mode};
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii};

const RECENT_KEY: &str = "kchess:palette-recent";
const RECENT_KEPT: usize = 8;

type Run = Rc<dyn Fn(&mut Window, &mut App)>;

/// One entry the palette can run (`QuickCommand`).
#[derive(Clone)]
struct Command {
    id: String,
    label: String,
    icon: Icon,
    /// Dimmed text after the label.
    detail: Option<String>,
    /// Right-aligned state: "On", "Current", "In progress".
    hint: Option<String>,
    /// Words that find it but are not shown.
    keywords: &'static str,
    kbds: &'static str,
    run: Run,
    /// Switches the palette's mode instead of running.
    prefix: Option<&'static str>,
}

impl Command {
    fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        icon: Icon,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Command {
        Command {
            id: id.into(),
            label: label.into(),
            icon,
            detail: None,
            hint: None,
            keywords: "",
            kbds: "",
            run: Rc::new(run),
            prefix: None,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Command {
        self.detail = Some(detail.into());
        self
    }

    fn hint(mut self, hint: Option<impl Into<String>>) -> Command {
        self.hint = hint.map(Into::into);
        self
    }

    fn keywords(mut self, keywords: &'static str) -> Command {
        self.keywords = keywords;
        self
    }

    fn kbds(mut self, kbds: &'static str) -> Command {
        self.kbds = kbds;
        self
    }
}

/// A group of results: its heading, and each entry with the ranges of its label to highlight.
struct Group {
    label: Option<&'static str>,
    items: Vec<(Command, Vec<(usize, usize)>)>,
    score: f64,
}

fn group(label: Option<&'static str>, commands: Vec<Command>) -> Option<Group> {
    (!commands.is_empty()).then(|| Group {
        label,
        items: commands.into_iter().map(|c| (c, Vec::new())).collect(),
        score: 0.,
    })
}

fn searched(
    label: &'static str,
    text: &str,
    commands: Vec<Command>,
    limit: usize,
) -> Option<Group> {
    let mut ranked: Vec<(usize, Command, quick_pick::Match)> = commands
        .into_iter()
        .enumerate()
        .filter_map(|(index, command)| {
            let extra = format!(
                "{} {}",
                command.detail.clone().unwrap_or_default(),
                command.keywords
            );
            quick_pick::match_query(text, &command.label, &extra).map(|m| (index, command, m))
        })
        .collect();
    ranked.sort_by(|a, b| b.2.score.total_cmp(&a.2.score).then(a.0.cmp(&b.0)));
    ranked.truncate(limit);
    let score = ranked.first()?.2.score;
    Some(Group {
        label: Some(label),
        items: ranked.into_iter().map(|(_, c, m)| (c, m.ranges)).collect(),
        score,
    })
}

fn go(page: Page) -> impl Fn(&mut Window, &mut App) + 'static {
    move |window, cx| crate::shell::open(page, window, cx)
}

fn on_off(on: bool) -> Option<String> {
    Some(if on { "On" } else { "Off" }.into())
}

fn current(page: Page, cx: &App) -> Option<String> {
    (crate::shell::current_page(cx) == Some(page)).then(|| "Current".into())
}

/// The sidebar's pages, Settings, and the Puzzles and Practice tabs (`places`).
fn places(cx: &App) -> Vec<Command> {
    let pages: [(Page, &str, Icon, &'static str); 16] = [
        (Page::Dashboard, "Home", icon("house"), "start overview"),
        (
            Page::Computer,
            "Play with Computer",
            icon("monitor"),
            "stockfish bot ai engine vs",
        ),
        (
            Page::Local,
            "Over the board",
            icon("users-round"),
            "otb friend two players pass and play clock",
        ),
        (Page::Puzzles, "Puzzles", icon("puzzle"), ""),
        (Page::Practice, "Practice", icon("graduation-cap"), ""),
        (
            Page::Openings,
            "Openings",
            icon("book-open-text"),
            "eco gambit defense variation explorer repertoire",
        ),
        (
            Page::Analysis,
            "Analysis board",
            icon("microscope"),
            "engine evaluate position fen pgn",
        ),
        (
            Page::History,
            "Game history",
            icon("history"),
            "past games archive",
        ),
        (
            Page::Studies,
            "Studies",
            icon("library-big"),
            "pgn chapters notes",
        ),
        (
            Page::Editor,
            "Board editor",
            icon("pencil-ruler"),
            "setup position fen",
        ),
        (
            Page::Online,
            "Play on Lichess",
            icon("globe-2"),
            "lichess seek pairing",
        ),
        (Page::Tournaments, "Tournaments", icon("trophy"), ""),
        (Page::Watch, "Watch", icon("tv"), "tv broadcast"),
        (
            Page::Insights,
            "Lichess insights",
            icon("chart-pie"),
            "stats openings performance",
        ),
        (
            Page::Friends,
            "Following",
            icon("users"),
            "following players",
        ),
        (
            Page::Players,
            "Players",
            icon("user-search"),
            "lookup profile user",
        ),
    ];
    let mut list: Vec<Command> = pages
        .into_iter()
        .map(|(page, label, glyph, keywords)| {
            Command::new(format!("page:{page:?}"), label, glyph, go(page))
                .keywords(keywords)
                .hint(current(page, cx))
        })
        .collect();
    list.push(
        Command::new(
            "page:Settings",
            "Settings",
            icon("settings-2"),
            go(Page::Settings),
        )
        .keywords("preferences options")
        .kbds(if cfg!(target_os = "macos") {
            "⌘ ,"
        } else {
            "Ctrl ,"
        })
        .hint(current(Page::Settings, cx)),
    );
    let puzzles: [(&str, &str, Icon, &'static str); 5] = [
        (
            "train",
            "Puzzle training",
            icon("puzzle"),
            "tactics solve themes motifs mate patterns",
        ),
        ("daily", "Daily puzzle", icon("calendar-days"), "today"),
        (
            "rush",
            "Puzzle Storm, Streak and Rush",
            icon("zap"),
            "timed race survival",
        ),
        (
            "stats",
            "Puzzle stats",
            icon("chart-column"),
            "rating progress",
        ),
        (
            "history",
            "Puzzle history",
            icon("history"),
            "solved attempts",
        ),
    ];
    for (tab, label, glyph, keywords) in puzzles {
        list.push(
            Command::new(format!("puzzles:{tab}"), label, glyph, move |window, cx| {
                crate::prefs::set(cx, "kchess:puzzle-tab", tab);
                crate::shell::open(Page::Puzzles, window, cx);
            })
            .detail("Puzzles")
            .keywords(keywords),
        );
    }
    let practice: [(&str, &str, Icon, &'static str); 6] = [
        (
            "coordinates",
            "Board coordinates",
            icon("grid-3x3"),
            "squares names",
        ),
        ("colors", "Square colours", icon("contrast"), "light dark"),
        ("knight", "Knight paths", icon("crown"), "knight tour"),
        (
            "endgames",
            "Endgame drills",
            icon("swords"),
            "mate technique",
        ),
        (
            "openings",
            "Repertoire trainer",
            icon("book-marked"),
            "opening drill study",
        ),
        (
            "mistakes",
            "Your mistakes",
            icon("book-open"),
            "blunders review",
        ),
    ];
    for (tab, label, glyph, keywords) in practice {
        list.push(
            Command::new(
                format!("practice:{tab}"),
                label,
                glyph,
                move |window, cx| {
                    crate::prefs::set(cx, "kchess:practice-tab", tab);
                    crate::shell::open(Page::Practice, window, cx);
                },
            )
            .detail("Practice")
            .keywords(keywords),
        );
    }
    list
}

fn setting(
    id: &'static str,
    label: &'static str,
    glyph: Icon,
    keywords: &'static str,
    on: bool,
    change: fn(&mut Settings),
) -> Command {
    Command::new(format!("setting:{id}"), label, glyph, move |_, cx| {
        crate::settings::change(cx, change)
    })
    .keywords(keywords)
    .hint(on_off(on))
}

/// Settings toggles and choices, and the Settings sections (`settingsCommands`).
fn settings_commands(cx: &App) -> Vec<Command> {
    let settings = cx.global::<SettingsHandle>().0.read(cx).settings.clone();
    let front = &settings.frontend;
    let mut list: Vec<Command> = [
        (
            Appearance::System,
            "Appearance: Match system",
            icon("sun-moon"),
        ),
        (Appearance::Light, "Appearance: Light", icon("sun")),
        (Appearance::Dark, "Appearance: Dark", icon("moon")),
    ]
    .into_iter()
    .map(|(appearance, label, glyph)| {
        Command::new(
            format!("setting:appearance:{appearance:?}"),
            label,
            glyph,
            move |_, cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance),
        )
        .keywords("theme mode color scheme")
        .hint((front.appearance == appearance).then_some("Current"))
    })
    .collect();
    list.extend([
        setting(
            "soundEnabled",
            "Sound",
            icon("volume-2"),
            "mute audio",
            front.sound_enabled,
            |s| s.frontend.sound_enabled = !s.frontend.sound_enabled,
        ),
        setting(
            "zenMode",
            "Zen mode",
            icon("minimize"),
            "focus distraction",
            front.zen_mode,
            |s| s.frontend.zen_mode = !s.frontend.zen_mode,
        )
        .kbds("Z"),
        setting(
            "blindfold",
            "Blindfold",
            icon("eye-off"),
            "hide pieces",
            front.blindfold,
            |s| s.frontend.blindfold = !s.frontend.blindfold,
        ),
        setting(
            "showLegalMoves",
            "Show legal moves",
            icon("locate"),
            "dots hints",
            front.show_legal_moves,
            |s| s.frontend.show_legal_moves = !s.frontend.show_legal_moves,
        ),
        setting(
            "premove",
            "Premoves",
            icon("fast-forward"),
            "queue move",
            front.premove,
            |s| s.frontend.premove = !s.frontend.premove,
        ),
        setting(
            "showOpeningName",
            "Show opening name",
            icon("book-marked"),
            "eco",
            front.show_opening_name,
            |s| s.frontend.show_opening_name = !s.frontend.show_opening_name,
        ),
        setting(
            "cloudEval",
            "Lichess cloud evaluation",
            icon("cloud"),
            "analysis engine",
            settings.core.cloud_eval,
            |s| s.core.cloud_eval = !s.core.cloud_eval,
        ),
        setting(
            "notificationsEnabled",
            "Notifications",
            icon("bell"),
            "alerts",
            front.notifications_enabled,
            |s| s.frontend.notifications_enabled = !s.frontend.notifications_enabled,
        ),
        setting(
            "swipeNavigation",
            "Two-finger swipe",
            icon("hand"),
            "gesture trackpad back forward",
            front.swipe_navigation,
            |s| s.frontend.swipe_navigation = !s.frontend.swipe_navigation,
        ),
    ]);
    for (mode, label) in [
        (CoordinateMode::None, "Coordinates: Hidden"),
        (CoordinateMode::Inside, "Coordinates: Inside the board"),
        (CoordinateMode::Outside, "Coordinates: Outside the board"),
    ] {
        list.push(
            Command::new(
                format!("setting:coordinates:{mode:?}"),
                label,
                icon("grid-3x3"),
                move |_, cx| crate::settings::change(cx, move |s| s.frontend.coordinates = mode),
            )
            .keywords("board labels files ranks")
            .hint((front.coordinates == mode).then_some("Current")),
        );
    }
    for (section, label, glyph) in crate::pages::settings::sections() {
        list.push(
            Command::new(
                format!("settings:{section:?}"),
                label,
                glyph,
                move |window, cx| crate::shell::open_settings(section, window, cx),
            )
            .detail("Settings")
            .hint(
                (section == Section::Updates)
                    .then(|| crate::updates::client_version(&crate::updates::current_version())),
            )
            .keywords(if section == Section::Updates {
                "version about upgrade"
            } else {
                ""
            }),
        );
    }
    list
}

/// Actions for the page in view, first in command mode (`pageCommands`).
fn page_commands(cx: &App) -> Vec<Command> {
    let zen = cx
        .global::<SettingsHandle>()
        .0
        .read(cx)
        .settings
        .frontend
        .zen_mode;
    let zen = setting(
        "zenMode",
        "Zen mode",
        icon("minimize"),
        "focus distraction",
        zen,
        |s| s.frontend.zen_mode = !s.frontend.zen_mode,
    )
    .kbds("Z");
    match crate::shell::current_page(cx) {
        Some(Page::Computer | Page::Local | Page::Online) => vec![zen],
        _ => Vec::new(),
    }
}

/// Stockfish at each level and side, online games, a new analysis board (`playCommands`).
fn play_commands(cx: &App) -> Vec<Command> {
    let levels: Vec<String> = cx
        .global::<SettingsHandle>()
        .0
        .read(cx)
        .settings
        .core
        .engine_levels
        .iter()
        .filter_map(|level| {
            serde_json::to_value(level)
                .ok()?
                .as_str()
                .map(str::to_string)
        })
        .collect();
    let mut list = Vec::new();
    for level in levels {
        let info = crate::game::rules("engineLevelInfo", &[json!(level)]).unwrap_or_default();
        let name = info["label"].as_str().unwrap_or(&level).to_string();
        let strength = info["elo"]
            .as_f64()
            .map_or("Full strength".to_string(), |elo| format!("~{elo}"));
        for (color, color_label) in [
            ("white", "White"),
            ("black", "Black"),
            ("random", "Random side"),
        ] {
            let level = level.clone();
            list.push(
                Command::new(
                    format!("play:stockfish:{level}:{color}"),
                    format!("Play Stockfish: {name}"),
                    icon("cpu"),
                    move |window, cx| crate::shell::play_computer(&level, color, window, cx),
                )
                .detail(format!("{strength} · {color_label}"))
                .keywords("computer bot engine ai"),
            );
        }
    }
    if !crate::accounts::handle(cx).read(cx).connected().is_empty() {
        for (minutes, increment) in [
            (3., 2.),
            (5., 0.),
            (10., 0.),
            (10., 5.),
            (15., 10.),
            (30., 0.),
        ] {
            let allowed = crate::game::rules(
                "canPlayOnline",
                &[json!(minutes), json!(increment), json!(false)],
            )
            .ok()
                == Some(json!(true));
            if !allowed {
                continue;
            }
            let perf = crate::game::rules("perfFor", &[json!(minutes), json!(increment)])
                .ok()
                .and_then(|perf| perf.as_str().map(str::to_string))
                .unwrap_or_default();
            for rated in [false, true] {
                list.push(
                    Command::new(
                        format!(
                            "play:online:{minutes}+{increment}:{}",
                            if rated { "rated" } else { "casual" }
                        ),
                        format!("Play online: {minutes}+{increment}"),
                        icon("globe-2"),
                        move |window, cx| {
                            crate::shell::play_online(minutes, increment, rated, window, cx)
                        },
                    )
                    .detail(format!(
                        "{perf} · {}",
                        if rated { "Rated" } else { "Casual" }
                    ))
                    .keywords("lichess seek opponent"),
                );
            }
        }
    }
    list.push(
        Command::new(
            "play:analysis",
            "New analysis board",
            icon("microscope"),
            crate::shell::new_analysis,
        )
        .keywords("empty start position"),
    );
    list
}

/// Syncing or connecting Lichess, and the sidebar (`appCommands`).
fn app_commands(cx: &App) -> Vec<Command> {
    let connected = !crate::accounts::handle(cx).read(cx).connected().is_empty();
    let sidebar = crate::shell::sidebar_open(cx);
    vec![
        if connected {
            Command::new(
                "app:sync",
                "Sync Lichess games",
                icon("refresh-cw"),
                |window, cx| {
                    crate::accounts::handle(cx)
                        .update(cx, |accounts, cx| accounts.sync(None, window, cx))
                },
            )
            .keywords("download update import")
        } else {
            Command::new(
                "app:connect",
                "Connect Lichess account",
                icon("log-in"),
                |window, cx| {
                    crate::accounts::handle(cx)
                        .update(cx, |accounts, cx| accounts.connect(window, cx))
                },
            )
            .keywords("sign in login oauth online")
        },
        Command::new(
            "app:sidebar",
            if sidebar {
                "Hide sidebar"
            } else {
                "Show sidebar"
            },
            icon("panel-left"),
            crate::shell::toggle_sidebar,
        )
        .keywords("toggle navigation"),
    ]
}

/// Followed players' profiles and challenges (`playerCommands`).
fn player_commands(cx: &App) -> Vec<Command> {
    let accounts = crate::accounts::handle(cx).read(cx);
    let connected = !accounts.connected().is_empty();
    let mut list = Vec::new();
    for friend in accounts.tracked() {
        let Some(name) = friend["username"].as_str().map(str::to_string) else {
            continue;
        };
        let profile = name.clone();
        list.push(
            Command::new(
                format!("player:{name}"),
                format!("@{name}"),
                icon("user-round"),
                move |window, cx| crate::shell::open_player(&profile, window, cx),
            )
            .detail("Profile")
            .keywords("following friend"),
        );
        if connected {
            for rated in [false, true] {
                let target = name.clone();
                list.push(
                    Command::new(
                        format!(
                            "challenge:{name}:{}",
                            if rated { "rated" } else { "casual" }
                        ),
                        format!("Challenge @{name}"),
                        icon("swords"),
                        move |window, cx| crate::shell::challenge_rated(&target, rated, window, cx),
                    )
                    .detail(if rated { "Rated" } else { "Casual" })
                    .keywords("play game invite"),
                );
            }
        }
    }
    list
}

/// Something worth returning to, above the recent entries (`resume`).
fn resume(cx: &App) -> Vec<Command> {
    let online = crate::online::handle(cx).read(cx);
    let mut list = Vec::new();
    if online.phase == Phase::Playing && crate::shell::current_page(cx) != Some(Page::Online) {
        list.push(
            Command::new(
                "resume:online",
                "Return to your online game",
                icon("globe-2"),
                go(Page::Online),
            )
            .hint(Some("In progress")),
        );
    }
    list
}

/// The palette's shortcuts as the keyboard has them: ⌘ on macOS, Ctrl elsewhere.
const SEARCH_KEYS: &str = if cfg!(target_os = "macos") {
    "⌘ K"
} else {
    "Ctrl K"
};
const COMMAND_KEYS: &str = if cfg!(target_os = "macos") {
    "⇧ ⌘ P"
} else {
    "Ctrl ⇧ P"
};

/// The prefixes, as rows that switch the mode (`MODE_ROWS`).
fn modes(include_go: bool) -> Vec<Command> {
    let rows: [(&'static str, &str, Icon, &'static str); 5] = [
        ("", "Go to page", icon("arrow-right"), SEARCH_KEYS),
        (">", "Run a command", icon("terminal"), COMMAND_KEYS),
        ("@", "Find a player", icon("at-sign"), ""),
        ("#", "Change a setting", icon("sliders-horizontal"), ""),
        ("?", "Help", icon("circle-help"), ""),
    ];
    rows.into_iter()
        .filter(|(prefix, ..)| (include_go || !prefix.is_empty()) && *prefix != "?")
        .map(|(prefix, label, glyph, kbds)| {
            let mut command = Command::new(format!("mode:{prefix}"), label, glyph, |_, _| {});
            if !prefix.is_empty() {
                command = command.detail(prefix);
            }
            command.prefix = Some(prefix);
            command.kbds(kbds)
        })
        .collect()
}

/// The palette's groups for what is typed (`groups`).
fn groups(query: &str, cx: &App) -> Vec<Group> {
    let (mode, text) = quick_pick::parse(query);
    match mode {
        Mode::Help => group(None, modes(true)).into_iter().collect(),
        Mode::Commands if text.is_empty() => [
            group(Some("This page"), page_commands(cx)),
            group(
                Some("Play"),
                play_commands(cx)
                    .into_iter()
                    .filter(|c| !c.id.starts_with("play:stockfish") || c.id.ends_with(":white"))
                    .collect(),
            ),
            group(Some("App"), app_commands(cx)),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Mode::Commands => [
            searched("This page", text, page_commands(cx), 8),
            searched("Play", text, play_commands(cx), 12),
            searched("App", text, app_commands(cx), 8),
            searched("Settings", text, settings_commands(cx), 8),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Mode::Settings if text.is_empty() => {
            let all = settings_commands(cx);
            let (sections, toggles): (Vec<Command>, Vec<Command>) =
                all.into_iter().partition(|c| c.id.starts_with("settings:"));
            [
                group(Some("Quick settings"), toggles),
                group(Some("Settings pages"), sections),
            ]
            .into_iter()
            .flatten()
            .collect()
        }
        Mode::Settings => searched("Settings", text, settings_commands(cx), 20)
            .into_iter()
            .collect(),
        Mode::Players => {
            let players = player_commands(cx);
            let mut out = Vec::new();
            let none = players.is_empty();
            if text.is_empty() {
                out.extend(group(Some("Following"), players));
                if none {
                    out.extend(group(
                        None,
                        vec![Command::new(
                            "page:Players",
                            "Type a Lichess username to look it up",
                            icon("user-search"),
                            go(Page::Players),
                        )],
                    ));
                }
            } else {
                out.extend(searched("Following", text, players, 12));
                let name = text.trim_start_matches('@').to_string();
                out.extend(group(
                    None,
                    vec![Command::new(
                        format!("lookup:{text}"),
                        format!("Look up “{text}” on Lichess"),
                        icon("user-search"),
                        move |window, cx| crate::shell::open_player(&name, window, cx),
                    )],
                ));
            }
            out
        }
        Mode::All if text.is_empty() => {
            let resume = resume(cx);
            let all: Vec<Command> = [
                places(cx),
                page_commands(cx),
                play_commands(cx),
                app_commands(cx),
                settings_commands(cx),
                player_commands(cx),
            ]
            .concat();
            let recent: Vec<String> = crate::prefs::get(cx, RECENT_KEY, Vec::new());
            let recent: Vec<Command> = recent
                .iter()
                .filter_map(|id| all.iter().find(|c| c.id == *id).cloned())
                .filter(|c| !resume.iter().any(|r| r.id == c.id))
                .take(6)
                .enumerate()
                .map(|(index, command)| {
                    if index == 0 && command.hint.is_none() {
                        command.hint(Some("recently used"))
                    } else {
                        command
                    }
                })
                .collect();
            let pages: Vec<Command> = places(cx)
                .into_iter()
                .filter(|c| c.id.starts_with("page:"))
                .collect();
            let had_recent = !recent.is_empty();
            [
                group(None, modes(false)),
                group(None, resume),
                group(None, recent),
                group(if had_recent { Some("Pages") } else { None }, pages),
            ]
            .into_iter()
            .flatten()
            .collect()
        }
        Mode::All => {
            let mut results: Vec<Group> = [
                searched("Go to", text, places(cx), 8),
                searched("This page", text, page_commands(cx), 8),
                searched("Play", text, play_commands(cx), 8),
                searched("Commands", text, app_commands(cx), 8),
                searched("Settings", text, settings_commands(cx), 8),
                searched("Following", text, player_commands(cx), 8),
            ]
            .into_iter()
            .flatten()
            .collect();
            results.sort_by(|a, b| b.score.total_cmp(&a.score));
            results
        }
    }
}

pub struct CommandPalette {
    input: Entity<InputState>,
    open: bool,
    selected: usize,
    _subscriptions: Vec<Subscription>,
}

/// The palette, from the shortcuts.
pub struct PaletteHandle(pub Entity<CommandPalette>);

impl Global for PaletteHandle {}

fn placeholder(mode: Mode, open: bool) -> &'static str {
    if !open {
        return "Search…";
    }
    match mode {
        Mode::All => "Search pages and commands (type ? for help)",
        Mode::Commands => "Type a command…",
        Mode::Players => "Search players you follow, or type a Lichess username",
        Mode::Settings => "Search settings…",
        Mode::Help => "Pick how to search",
    }
}

impl CommandPalette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> CommandPalette {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search…"));
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Focus => {
                    this.open = true;
                    this.refresh_placeholder(window, cx);
                    cx.notify();
                }
                // Anything else taking the keys closes it, as VS Code's quick pick does.
                InputEvent::Blur => {
                    this.open = false;
                    this.refresh_placeholder(window, cx);
                    cx.notify();
                }
                // Typing opens it; text cleared on closing does not.
                InputEvent::Change => {
                    this.selected = 0;
                    if input.read(cx).focus_handle(cx).is_focused(window) {
                        this.open = true;
                    }
                    this.refresh_placeholder(window, cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.run_selected(window, cx),
            },
        )];
        CommandPalette {
            input,
            open: false,
            selected: 0,
            _subscriptions: subscriptions,
        }
    }

    fn refresh_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        let text = placeholder(quick_pick::parse(&query).0, self.open);
        self.input
            .update(cx, |input, cx| input.set_placeholder(text, window, cx));
    }

    /// ⌘K, or ⇧⌘P with `>`: open with `prefix` typed.
    pub fn show(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.selected = 0;
        let prefix = prefix.to_string();
        self.input.update(cx, |input, cx| {
            input.set_value(prefix, window, cx);
            input.focus(window, cx);
        });
        self.refresh_placeholder(window, cx);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.selected = 0;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.blur(cx);
        self.refresh_placeholder(window, cx);
        cx.notify();
    }

    fn entries(&self, cx: &App) -> Vec<Command> {
        let query = self.input.read(cx).value().to_string();
        groups(&query, cx)
            .into_iter()
            .flat_map(|group| group.items.into_iter().map(|(command, _)| command))
            .collect()
    }

    fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        if let Some(command) = entries.get(self.selected.min(entries.len().saturating_sub(1))) {
            self.run(command.clone(), window, cx);
        }
    }

    fn run(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prefix) = command.prefix {
            self.show(prefix, window, cx);
            return;
        }
        let recent: Vec<String> = crate::prefs::get(cx, RECENT_KEY, Vec::new());
        crate::prefs::set(
            cx,
            RECENT_KEY,
            quick_pick::push_recent(&recent, &command.id, RECENT_KEPT),
        );
        self.close(window, cx);
        let run = command.run.clone();
        window.defer(cx, move |window, cx| run(window, cx));
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" if self.open => {
                self.close(window, cx);
                cx.stop_propagation();
            }
            "down" => {
                let count = self.entries(cx).len();
                if count > 0 {
                    self.selected = (self.selected + 1) % count;
                }
                cx.stop_propagation();
                cx.notify();
            }
            "up" => {
                let count = self.entries(cx).len();
                if count > 0 {
                    self.selected = (self.selected + count - 1) % count;
                }
                cx.stop_propagation();
                cx.notify();
            }
            // Backspace on an empty mode drops back to plain search.
            "backspace" => {
                let query = self.input.read(cx).value().to_string();
                if query.chars().count() == 1 && quick_pick::parse(&query).0 != Mode::All {
                    self.input
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    self.refresh_placeholder(window, cx);
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }
}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let open = self.open;
        let query = self.input.read(cx).value().to_string();
        let groups = if open { groups(&query, cx) } else { Vec::new() };
        let count: usize = groups.iter().map(|g| g.items.len()).sum();
        let selected = self.selected.min(count.saturating_sub(1));
        let (_, text) = quick_pick::parse(&query);
        let mut index = 0usize;
        let results = open.then(|| {
            let sections: Vec<AnyElement> = groups
                .into_iter()
                .enumerate()
                .map(|(at_group, group)| {
                    let heading = group.label.map(|label| {
                        div()
                            .px(px(10.))
                            .pt(px(6.))
                            .pb(px(4.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.text_muted)
                            .child(label)
                    });
                    let items: Vec<AnyElement> = group
                        .items
                        .into_iter()
                        .map(|(command, ranges)| {
                            let at = index;
                            index += 1;
                            let chosen = at == selected;
                            let segments = quick_pick::segments(&command.label, &ranges);
                            let run = command.clone();
                            h_flex()
                                .id(("palette-item", at))
                                .test_support()
                                .role(Role::Button)
                                .aria_label(command.label.clone())
                                .aria_selected(chosen)
                                .gap(px(10.))
                                .px(px(10.))
                                .py(px(6.))
                                .rounded(radii::SM)
                                .text_size(px(13.))
                                .when(chosen, |row| {
                                    row.bg(crate::theme::tint(palette.primary, 12.))
                                })
                                .cursor_pointer()
                                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                    if this.selected != at {
                                        this.selected = at;
                                        cx.notify();
                                    }
                                }))
                                // Down, not click: the box loses the focus (and closes) before a click lands.
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.run(run.clone(), window, cx)
                                    }),
                                )
                                .child(command.icon.clone().size(px(16.)).text_color(if chosen {
                                    palette.primary
                                } else {
                                    palette.text_muted
                                }))
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(8.))
                                        .child(h_flex().children(segments.into_iter().map(
                                            |(part, mark)| {
                                                div()
                                                    .when(mark, |part| {
                                                        part.text_color(palette.primary)
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                    })
                                                    .child(part)
                                            },
                                        )))
                                        .children(command.detail.map(|detail| {
                                            div()
                                                .text_size(px(12.))
                                                .text_color(palette.dimmed)
                                                .truncate()
                                                .child(detail)
                                        })),
                                )
                                .children(command.hint.map(|hint| {
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.dimmed)
                                        .child(hint)
                                }))
                                .when(!command.kbds.is_empty(), |row| {
                                    row.child(kbd(command.kbds, &palette))
                                })
                                .into_any_element()
                        })
                        .collect();
                    // `.kchess-quickpick-group`: a rule between groups.
                    v_flex()
                        .py(px(4.))
                        .when(at_group > 0, |group| {
                            group.border_t_1().border_color(palette.border)
                        })
                        .children(heading)
                        .children(items)
                        .into_any_element()
                })
                .collect();
            let empty = sections.is_empty();
            let list = v_flex()
                .id("palette-results")
                .min_h_0()
                .flex_shrink_1()
                .overflow_y_scroll()
                .px(px(4.))
                .border_t_1()
                .border_color(palette.border)
                .when(empty, |list| {
                    list.child(
                        div()
                            .py(px(24.))
                            .text_center()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(format!("No results for “{text}”")),
                    )
                })
                .children(sections);
            let hint = |keys: &'static str, label: &'static str| {
                h_flex().gap(px(3.)).child(kbd(keys, &palette)).child(label)
            };
            let footer = h_flex()
                .flex_none()
                .flex_wrap()
                .gap_x(px(14.))
                .gap_y(px(4.))
                .px(px(10.))
                .py(px(6.))
                .border_t_1()
                .border_color(palette.border)
                .text_size(px(11.))
                .text_color(palette.text_muted)
                .child(hint("↑ ↓", "navigate"))
                .child(hint("↵", "open"))
                .child(hint("esc", "close"))
                .child(
                    h_flex()
                        .ml_auto()
                        .gap(px(3.))
                        .child(kbd(">", &palette))
                        .child("commands")
                        .child(kbd("@", &palette))
                        .child("players")
                        .child(kbd("#", &palette))
                        .child("settings"),
                );
            (list, footer)
        });
        // `.quickpick-keys` hides below 560px.
        let shortcut = !open && window.viewport_size().width >= px(560.);
        let input = self.input.clone();
        let field = h_flex()
            .flex_none()
            .h(px(30.))
            .px(px(10.))
            .gap(px(6.))
            .when(!open, |field| field.cursor_pointer())
            // A press anywhere on the closed box opens it, the icon and shortcut included.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !open {
                    input.update(cx, |input, cx| input.focus(window, cx));
                }
            })
            .child(
                icon("search")
                    .size(px(16.))
                    .flex_none()
                    .text_color(palette.dimmed),
            )
            .child(
                Input::new(&self.input)
                    .appearance(false)
                    .flex_1()
                    .h(px(30.))
                    .px_0()
                    .text_size(px(13.))
                    .aria_label("Search pages and commands"),
            )
            .when(shortcut, |field| field.child(kbd(SEARCH_KEYS, &palette)));
        // `.quickpick-panel`: one bordered box, the input on top; open, it grows down over the
        // page with the results and keys beneath, up to 560px.
        let tall = (window.viewport_size().height - px(16.)).min(px(560.));
        let panel = v_flex()
            .id("palette-panel")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .rounded(radii::MD)
            .border_1()
            .border_color(palette.border_accented)
            .bg(if open && palette.dark {
                palette.elevated
            } else {
                palette.bg
            })
            .overflow_hidden()
            .when(!open, |panel| {
                panel.hover(|style| style.bg(palette.elevated))
            })
            .when(open, |panel| {
                panel
                    .max_h(tall)
                    .occlude()
                    .shadow(vec![BoxShadow {
                        color: hsla(0., 0., 0., 0.35),
                        offset: point(px(0.), px(18.)),
                        blur_radius: px(40.),
                        spread_radius: px(-12.),
                        inset: false,
                    }])
                    // A press beside it closes it too, where nothing takes the focus (the
                    // title bar's empty sides); a press that does is left to `Blur`.
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                        let input = this.input.clone();
                        cx.defer_in(window, move |this, window, cx| {
                            if this.open && input.read(cx).focus_handle(cx).is_focused(window) {
                                this.close(window, cx);
                            }
                        });
                    }))
            })
            .child(field)
            .when_some(results, |panel, (list, footer)| {
                panel.child(list).child(footer)
            });
        div()
            .relative()
            .w_full()
            .h(px(32.))
            .capture_key_down(cx.listener(Self::key))
            // Drawn after the page, so the open results lie over it.
            .child(deferred(panel).with_priority(1))
    }
}

/// A shortcut's keys as `UKbd` chips (size sm).
fn kbd(keys: &'static str, palette: &Palette) -> impl IntoElement {
    h_flex()
        .flex_none()
        .gap(px(2.))
        .children(keys.split(' ').map(|key| {
            h_flex()
                .h(px(20.))
                .min_w(px(20.))
                .px(px(4.))
                .justify_center()
                .rounded(radii::XS)
                .border_1()
                .border_color(palette.border_accented)
                .bg(palette.bg)
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(key.to_uppercase())
        }))
}

pub fn init(window: &mut Window, cx: &mut App) -> Entity<CommandPalette> {
    let palette = cx.new(|cx| CommandPalette::new(window, cx));
    cx.set_global(PaletteHandle(palette.clone()));
    palette
}
