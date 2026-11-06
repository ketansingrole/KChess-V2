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

/// Each command once, in the first group that has it: a page's own toggle (Zen mode) is also a
/// setting, and searching listed it twice.
fn once_each(groups: Vec<Group>) -> Vec<Group> {
    let mut seen = std::collections::HashSet::new();
    groups
        .into_iter()
        .filter_map(|mut group| {
            group
                .items
                .retain(|(command, _)| seen.insert(command.id.clone()));
            (!group.items.is_empty()).then_some(group)
        })
        .collect()
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

/// The sidebar's pages, Settings, and the Puzzles, Watch, Game history and Practice tabs
/// (`places`).
fn places(cx: &App) -> Vec<Command> {
    let pages: [(Page, &str, Icon, &'static str); 16] = [
        (Page::Dashboard, "Home", icon("house"), "start overview"),
        (
            Page::Computer,
            "Play with computer",
            icon("bot"),
            "stockfish bot ai engine vs",
        ),
        (
            Page::Local,
            "Over the board",
            icon("swords"),
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
    list.push(
        Command::new(
            "history:lichess",
            "Lichess games",
            icon("history"),
            crate::shell::open_history_lichess,
        )
        .detail("Game history")
        .keywords("synced online played"),
    );
    let watch: [(&str, &str, Icon, &'static str); 3] = [
        ("tv", "Lichess TV", icon("tv"), "channels top rated"),
        (
            "friends",
            "Following games",
            icon("users"),
            "friends players live",
        ),
        (
            "broadcasts",
            "Broadcasts",
            icon("radio"),
            "over the board events rounds",
        ),
    ];
    for (tab, label, glyph, keywords) in watch {
        list.push(
            Command::new(format!("watch:{tab}"), label, glyph, move |window, cx| {
                crate::shell::watch_tab(tab, window, cx)
            })
            .detail("Watch")
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
            .keywords(crate::pages::settings::section_keywords(section)),
        );
    }
    list
}

/// Actions for the page in view, first in command mode (`pageCommands`), with ⌘C and ⌘N shown
/// beside the commands those keys run on this page.
fn page_commands(cx: &App) -> Vec<Command> {
    let mut commands = page_commands_unkeyed(cx);
    let (copy, new) = if cfg!(target_os = "macos") {
        ("⌘ C", "⌘ N")
    } else {
        ("Ctrl C", "Ctrl N")
    };
    if let Some(at) = copy_target(&commands) {
        commands[at].kbds = copy;
    }
    if let Some(at) = new_target(&commands) {
        commands[at].kbds = new;
    }
    commands
}

/// Which of a page's commands Copy outside a text field runs: its game's PGN, or its position's
/// FEN where it has no game.
fn copy_target(commands: &[Command]) -> Option<usize> {
    [":copy-pgn", ":copy-fen"].into_iter().find_map(|kind| {
        commands
            .iter()
            .position(|command| command.id.ends_with(kind))
    })
}

/// Which of a page's commands New runs.
fn new_target(commands: &[Command]) -> Option<usize> {
    commands
        .iter()
        .position(|command| command.id.ends_with(":new-game") || command.id == "studies:new")
}

/// The page's keys beside its commands, by id (`page_commands`).
#[cfg(test)]
pub(crate) fn page_command_keys(cx: &App) -> Vec<(String, &'static str)> {
    page_commands(cx)
        .into_iter()
        .filter(|command| !command.kbds.is_empty())
        .map(|command| (command.id.clone(), command.kbds))
        .collect()
}

fn page_commands_unkeyed(cx: &App) -> Vec<Command> {
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
        Some(Page::Computer) => std::iter::once(zen).chain(computer_commands(cx)).collect(),
        Some(Page::Local) => std::iter::once(zen).chain(local_commands(cx)).collect(),
        Some(Page::Online) => std::iter::once(zen).chain(live_commands(cx)).collect(),
        Some(Page::Analysis) => analysis_commands()
            .into_iter()
            .chain(study_commands(cx))
            .collect(),
        Some(Page::Editor) => editor_commands(cx),
        Some(Page::Puzzles) => puzzle_commands(cx),
        Some(Page::Watch) => watch_commands(cx),
        Some(Page::Studies) => studies_commands(),
        Some(Page::Openings) => openings_commands(cx),
        Some(Page::Tournaments) => tournament_commands(cx),
        Some(Page::Practice) => practice_commands(cx),
        Some(Page::History) => history_commands(cx),
        Some(Page::Players) => profile_commands(cx),
        Some(Page::Friends) => following_commands(cx),
        Some(Page::Dashboard) => home_commands(cx),
        _ => Vec::new(),
    }
}

/// Every command Quick Pick can offer now, by id and label: recent entries are found by id.
#[cfg(test)]
pub(crate) fn command_ids(cx: &App) -> Vec<(String, String)> {
    [
        places(cx),
        page_commands(cx),
        play_commands(cx),
        app_commands(cx),
        settings_commands(cx),
        player_commands(cx),
    ]
    .concat()
    .into_iter()
    .map(|command| (command.id.clone(), command.label.clone()))
    .collect()
}

/// How Quick Pick names a page's action: its id, label, icon and search keywords.
type Described = (&'static str, &'static str, &'static str, &'static str);

/// A page's actions as commands, each run by `act` on that page if it is still open.
fn page_actions<V: 'static, A: Copy + 'static>(
    page: Page,
    prefix: &'static str,
    actions: Vec<A>,
    describe: fn(A) -> Described,
    act: fn(&mut V, A, &mut Window, &mut Context<V>),
) -> Vec<Command> {
    actions
        .into_iter()
        .map(|action| {
            let (id, label, glyph, keywords) = describe(action);
            Command::new(
                format!("{prefix}:{id}"),
                label,
                icon(glyph),
                move |window, cx| {
                    if let Some(view) = crate::shell::view_of::<V>(page, cx) {
                        view.update(cx, |view, cx| act(view, action, window, cx));
                    }
                },
            )
            .keywords(keywords)
        })
        .collect()
}

/// A page's flip command, with the F its page takes for it.
fn flip_key(command: Command) -> Command {
    if command.id.ends_with("flip") {
        command.kbds("F")
    } else {
        command
    }
}

/// The computer game's actions its buttons allow now.
fn computer_commands(cx: &App) -> Vec<Command> {
    use crate::pages::computer::{Action, ComputerPage};
    let Some(page) = crate::shell::view_of::<ComputerPage>(Page::Computer, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Computer,
        "computer",
        page.read(cx).actions(),
        |action| match action {
            Action::TakeBack => ("takeback", "Take back", "undo-2", "undo"),
            Action::Resign => ("resign", "Resign", "flag", "give up"),
            Action::NewGame => ("new-game", "New game", "rotate-ccw", "restart"),
            Action::SwitchSides => ("switch-sides", "Switch sides", "repeat", "colour color"),
            Action::CopyPgn => ("copy-pgn", "Copy PGN", "copy", "game moves clipboard"),
            Action::Review => ("review", "Review game", "microscope", "analyse analysis"),
        },
        ComputerPage::act,
    )
}

/// The live Lichess game's actions, as its buttons offer them.
fn live_commands(cx: &App) -> Vec<Command> {
    use crate::pages::online::game::{LiveAction, live_actions, run_live_action};
    live_actions(crate::online::handle(cx).read(cx))
        .into_iter()
        .map(|action| {
            let (id, label, glyph, keywords) = match action {
                LiveAction::Berserk => ("berserk", "Berserk", "flame", "arena half time"),
                LiveAction::Takeback => ("takeback", "Takeback", "undo-2", "undo take back"),
                LiveAction::OfferDraw => ("draw", "Offer draw", "handshake", "agree"),
                LiveAction::Resign => ("resign", "Resign", "flag", "give up"),
                LiveAction::Abort => ("abort", "Abort", "ban", "cancel"),
                LiveAction::Rematch => ("rematch", "Rematch", "repeat", "again"),
                LiveAction::NewOpponent => (
                    "new-opponent",
                    "New opponent",
                    "users",
                    "seek again same time control",
                ),
                LiveAction::Analyse => ("analyse", "Analyse", "microscope", "analysis review"),
                LiveAction::NewGame => ("new-game", "New game", "plus", "another seek"),
                LiveAction::CancelSeek => ("cancel-seek", "Cancel seek", "x", "stop waiting"),
                LiveAction::Flip => ("flip", "Flip board", "arrow-up-down", "turn rotate"),
            };
            Command::new(
                format!("online:{id}"),
                label,
                icon(glyph),
                move |window, cx| run_live_action(action, window, cx),
            )
            .keywords(keywords)
            .kbds(if action == LiveAction::Flip { "F" } else { "" })
        })
        .collect()
}

/// The shown puzzle's actions, as its panel's buttons offer them.
fn puzzle_commands(cx: &App) -> Vec<Command> {
    use crate::pages::puzzles::{Action, PuzzlesPage};
    let Some(page) = crate::shell::view_of::<PuzzlesPage>(Page::Puzzles, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Puzzles,
        "puzzle",
        page.read(cx).actions(cx),
        |action| match action {
            Action::Hint => ("hint", "Hint", "lightbulb", "clue help"),
            Action::Solution => ("solution", "Solution", "eye", "show answer give up"),
            Action::Flip => ("flip", "Flip board", "arrow-up-down", "turn rotate"),
            Action::Analyse => (
                "analyse",
                "Analysis board",
                "microscope",
                "analyse study review",
            ),
            Action::Next => ("next", "Next puzzle", "arrow-right", "another skip"),
            Action::Rush(action) => {
                use crate::pages::puzzles::RushAction;
                match action {
                    RushAction::Start => {
                        ("rush-start", "Start run", "play", "storm streak rush begin")
                    }
                    RushAction::Skip => ("rush-skip", "Skip puzzle", "skip-forward", "pass"),
                    RushAction::End => ("rush-end", "End run", "flag", "stop finish quit"),
                    RushAction::Again => {
                        ("rush-again", "Play again", "rotate-ccw", "restart retry")
                    }
                    RushAction::Back => ("rush-back", "Back to modes", "arrow-left", "menu"),
                }
            }
        },
        |page: &mut PuzzlesPage, action, window, cx| match action {
            Action::Analyse => page.analyse(window, cx),
            action => page.act(action, cx),
        },
    )
    .into_iter()
    .map(flip_key)
    .collect()
}

/// The watched game's actions, as its panel's buttons offer them.
fn watch_commands(cx: &App) -> Vec<Command> {
    use crate::pages::watch::{WatchPage, live::Action};
    let Some(page) = crate::shell::view_of::<WatchPage>(Page::Watch, cx) else {
        return Vec::new();
    };
    let channels = page
        .read(cx)
        .other_channels(cx)
        .into_iter()
        .map(|(key, name)| {
            Command::new(
                format!("watch:channel:{key}"),
                format!("Watch {name}"),
                icon("tv"),
                move |window, cx| crate::shell::watch_channel(key.clone(), window, cx),
            )
            .keywords("tv channel")
        });
    page_actions(
        Page::Watch,
        "watch",
        page.read(cx).actions(),
        |action| match action {
            Action::Flip => ("flip", "Flip board", "arrow-up-down", "turn rotate"),
            Action::AllBoards => ("all-boards", "All boards", "layout-grid", "round broadcast"),
            Action::Analyse => ("analyse", "Analyse", "microscope", "analysis"),
            Action::Stop => ("stop", "Stop watching", "square", "close end"),
        },
        WatchPage::act,
    )
    .into_iter()
    .map(flip_key)
    .chain(channels)
    .collect()
}

/// An open line's actions on the Openings page.
fn openings_commands(cx: &App) -> Vec<Command> {
    use crate::pages::openings::{Action, OpeningsPage};
    let Some(page) = crate::shell::view_of::<OpeningsPage>(Page::Openings, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Openings,
        "openings",
        page.read(cx).actions(),
        |action| match action {
            Action::Train => (
                "train",
                "Train this line",
                "graduation-cap",
                "repertoire drill",
            ),
            Action::Analyse => ("analyse", "Analyse", "microscope", "analysis"),
            Action::Puzzles => ("puzzles", "Train with puzzles", "puzzle", "tactics"),
            Action::Flip => ("flip", "Flip board", "arrow-up-down", "turn rotate"),
            Action::AllOpenings => ("all", "All openings", "arrow-left", "back browse"),
        },
        OpeningsPage::act,
    )
    .into_iter()
    .map(flip_key)
    .collect()
}

/// The Tournaments page's actions: the schedule's, or the open event's.
fn tournament_commands(cx: &App) -> Vec<Command> {
    use crate::pages::tournaments::{Action, TournamentsPage};
    let Some(page) = crate::shell::view_of::<TournamentsPage>(Page::Tournaments, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Tournaments,
        "tournaments",
        page.read(cx).actions(cx),
        |action| match action {
            Action::Refresh => ("refresh", "Refresh", "refresh-cw", "reload schedule"),
            Action::Create => ("create", "Create arena", "plus", "new tournament host"),
            Action::Back => ("back", "All tournaments", "arrow-left", "schedule list"),
            Action::Join => ("join", "Join", "log-in", "enter tournament"),
            Action::Game => ("game", "Go to your game", "swords", "play pairing"),
            Action::Pause => ("pause", "Pause", "pause", "leave arena"),
            Action::Withdraw => ("withdraw", "Withdraw", "log-out", "leave tournament"),
        },
        TournamentsPage::run,
    )
}

/// The actions of the Practice trainer on screen: an endgame lesson's or drill's, the
/// repertoire's or the mistakes'.
fn practice_commands(cx: &App) -> Vec<Command> {
    use crate::pages::practice::{
        Action, PracticePage, endgame_drills::DrillAction, endgame_lesson::LessonAction,
        mistakes::MistakeAction, repertoire::RepertoireAction,
    };
    let Some(page) = crate::shell::view_of::<PracticePage>(Page::Practice, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Practice,
        "practice",
        page.read(cx).actions(cx),
        |action| match action {
            Action::Drill(DrillAction::Hint) => ("hint", "Hint", "lightbulb", "help best move"),
            Action::Drill(DrillAction::TakeBack) => ("takeback", "Take back", "undo-2", "undo"),
            Action::Drill(DrillAction::Flip) => {
                ("flip", "Flip board", "arrow-up-down", "turn rotate")
            }
            Action::Drill(DrillAction::Restart) => ("restart", "Restart", "rotate-ccw", "again"),
            Action::Lesson(LessonAction::Back) => {
                ("courses", "All courses", "chevron-left", "back lessons")
            }
            Action::Lesson(LessonAction::Hint) => ("lesson-hint", "Hint", "lightbulb", "help"),
            Action::Lesson(LessonAction::TakeBack) => {
                ("lesson-takeback", "Take back", "undo-2", "undo")
            }
            Action::Lesson(LessonAction::Flip) => {
                ("lesson-flip", "Flip board", "arrow-up-down", "turn rotate")
            }
            Action::Lesson(LessonAction::Retry) => ("retry", "Retry", "rotate-ccw", "again"),
            Action::Lesson(LessonAction::Previous) => {
                ("previous", "Previous step", "chevron-left", "back")
            }
            Action::Lesson(LessonAction::Next) => ("next-step", "Next", "chevron-right", "step"),
            Action::Lesson(LessonAction::NextChapter) => {
                ("next-chapter", "Next chapter", "chevron-right", "lesson")
            }
            Action::Repertoire(RepertoireAction::ShowMove) => {
                ("show-move", "Show move", "lightbulb", "hint answer")
            }
            Action::Repertoire(RepertoireAction::Restart) => (
                "restart-line",
                "Restart line",
                "rotate-ccw",
                "new line again",
            ),
            Action::Repertoire(RepertoireAction::Forget) => {
                ("forget", "Forget mistakes", "eraser", "clear misses")
            }
            Action::Repertoire(RepertoireAction::Save) => {
                ("save", "Save as study", "save", "keep opening")
            }
            Action::Mistakes(MistakeAction::Hint) => {
                ("mistake-hint", "Hint: piece to move", "lightbulb", "help")
            }
            Action::Mistakes(MistakeAction::Solution) => {
                ("solution", "Show solution", "eye", "answer")
            }
            Action::Mistakes(MistakeAction::Next) => {
                ("next", "Next due position", "arrow-right", "mistake")
            }
            Action::Mistakes(MistakeAction::Due) => {
                ("due", "Practice due positions", "target", "mistakes start")
            }
            Action::Mistakes(MistakeAction::Early) => {
                ("early", "Review early", "fast-forward", "mistakes ahead")
            }
        },
        PracticePage::act,
    )
    .into_iter()
    .map(flip_key)
    .collect()
}

/// Game history's actions: the open game's, or the synced games'.
fn history_commands(cx: &App) -> Vec<Command> {
    use crate::pages::history::{Action, HistoryPage};
    let Some(page) = crate::shell::view_of::<HistoryPage>(Page::History, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::History,
        "history",
        page.read(cx).actions(cx),
        |action| match action {
            Action::AllGames => ("all", "All games", "arrow-left", "back list"),
            Action::Export => ("export", "Export PGN", "download", "save file"),
            Action::Analyse => ("analyse", "Open analysis", "chart-line", "analysis board"),
            Action::Review => ("review", "Review", "chart-line", "analysis accuracy"),
            Action::Sync => ("sync", "Sync games", "refresh-cw", "lichess download"),
            Action::ClearFilters => ("clear", "Clear filters", "x", "reset all"),
        },
        HistoryPage::act,
    )
}

/// The open player's actions on the Players page.
fn profile_commands(cx: &App) -> Vec<Command> {
    use crate::pages::players::{Action, PlayersPage};
    let Some(page) = crate::shell::view_of::<PlayersPage>(Page::Players, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Players,
        "players",
        page.read(cx).actions(cx),
        |action| match action {
            Action::Follow => ("follow", "Follow player", "user-plus", "friend add"),
            Action::Challenge => ("challenge", "Challenge", "swords", "play game invite"),
            Action::Message => ("message", "Message", "message-circle", "send write"),
            Action::Watch => ("watch", "Watch", "tv", "spectate game"),
        },
        PlayersPage::act,
    )
}

/// The Following page's header actions.
fn following_commands(cx: &App) -> Vec<Command> {
    use crate::pages::friends::{Action, FriendsPage};
    let Some(page) = crate::shell::view_of::<FriendsPage>(Page::Friends, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Friends,
        "following",
        page.read(cx).actions(cx),
        |action| match action {
            Action::Follow => ("follow", "Follow player", "plus", "friend add"),
            Action::Import => (
                "import",
                "Import from Lichess",
                "download",
                "followed players",
            ),
            Action::Sync => ("sync", "Sync all", "refresh-cw", "games download"),
        },
        FriendsPage::act,
    )
}

/// Home's Review: the last game.
fn home_commands(cx: &App) -> Vec<Command> {
    use crate::pages::home::{Action, HomePage};
    let Some(page) = crate::shell::view_of::<HomePage>(Page::Dashboard, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Dashboard,
        "home",
        page.read(cx).actions(cx),
        |action| match action {
            Action::Review => ("review", "Review last game", "microscope", "analyse recent"),
        },
        HomePage::act,
    )
}

/// The Studies page's header actions.
fn studies_commands() -> Vec<Command> {
    use crate::pages::studies::StudiesPage;
    vec![
        Command::new(
            "studies:new",
            "New study",
            icon("plus"),
            StudiesPage::create,
        )
        .keywords("create chapter"),
        Command::new(
            "studies:import",
            "Import PGN",
            icon("import"),
            StudiesPage::open_import,
        )
        .keywords("paste games study"),
    ]
}

/// The board editor's actions, as its buttons offer them.
fn editor_commands(cx: &App) -> Vec<Command> {
    use crate::pages::editor::{Action, EditorPage};
    let Some(page) = crate::shell::view_of::<EditorPage>(Page::Editor, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Editor,
        "editor",
        page.read(cx).actions(),
        |action| match action {
            Action::StartPosition => ("start", "Starting position", "rotate-ccw", "reset"),
            Action::ClearBoard => ("clear", "Clear board", "eraser", "empty"),
            Action::Flip => ("flip", "Flip board", "arrow-up-down", "turn rotate"),
            Action::CopyFen => ("copy-fen", "Copy FEN", "copy", "position clipboard"),
            Action::Analyse => ("analyse", "Analyse this position", "microscope", "analysis"),
            Action::PlayStockfish => ("stockfish", "Play Stockfish", "cpu", "computer"),
            Action::PlayFriend => (
                "friend",
                "Play a friend here",
                "users-round",
                "over the board local",
            ),
        },
        EditorPage::act,
    )
    .into_iter()
    .map(flip_key)
    .collect()
}

/// The game over the board's actions, as its panel's buttons offer them.
fn local_commands(cx: &App) -> Vec<Command> {
    use crate::pages::local::{Action, LocalPage};
    let Some(page) = crate::shell::view_of::<LocalPage>(Page::Local, cx) else {
        return Vec::new();
    };
    page_actions(
        Page::Local,
        "local",
        page.read(cx).actions(cx),
        |action| match action {
            Action::TakeBack => ("takeback", "Take back", "undo-2", "undo"),
            Action::Pause => ("pause", "Pause", "pause", "clock stop"),
            Action::Resume => ("resume", "Resume", "play", "clock continue"),
            Action::Draw => ("draw", "Draw", "handshake", "agree"),
            Action::Resign => ("resign", "Resign", "flag", "give up"),
            Action::NewGame => ("new-game", "New game", "plus", "restart"),
            Action::Flip => (
                "flip",
                "Flip board",
                "refresh-cw",
                "turn rotate orientation",
            ),
            Action::CopyPgn => ("copy-pgn", "Copy PGN", "copy", "game moves clipboard"),
            Action::Analyse => ("analyse", "Analyse", "microscope", "analysis review"),
        },
        LocalPage::act,
    )
    .into_iter()
    .map(flip_key)
    .collect()
}

/// The study bar's menus over the analysis board: the study's and its chapters' actions.
fn study_commands(cx: &App) -> Vec<Command> {
    use crate::pages::analysis::AnalysisPage;
    use crate::pages::study_bar::StudyAction;
    let Some(page) = crate::shell::view_of::<AnalysisPage>(Page::Analysis, cx) else {
        return Vec::new();
    };
    let bar = page.read(cx).study_bar();
    let cloud = bar.read(cx).cloud(cx);
    bar.read(cx)
        .actions(cx)
        .into_iter()
        .map(|action| {
            let (id, label, glyph, keywords) = match action {
                StudyAction::SaveAsStudy => ("save", "Save as study", "book-plus", "keep store"),
                StudyAction::Details => (
                    "details",
                    "Game details",
                    "file-pen-line",
                    "headers pgn tags",
                ),
                StudyAction::SaveCopy => ("copy", "Save a copy", "copy", "duplicate study"),
                StudyAction::Close => ("close", "Close study", "book-x", "unsaved analysis"),
                StudyAction::Remove if cloud => {
                    ("remove", "Remove offline copy", "trash-2", "lichess study")
                }
                StudyAction::Remove => ("remove", "Delete study", "trash-2", "remove"),
                StudyAction::AddChapter => ("add-chapter", "Add chapter", "plus", "new"),
                StudyAction::RenameChapter => {
                    ("rename-chapter", "Rename chapter", "pencil", "name")
                }
                StudyAction::DuplicateChapter => {
                    ("duplicate-chapter", "Duplicate chapter", "copy", "copy")
                }
                StudyAction::MoveChapterUp => (
                    "move-chapter-up",
                    "Move chapter up",
                    "chevron-up",
                    "reorder",
                ),
                StudyAction::MoveChapterDown => (
                    "move-chapter-down",
                    "Move chapter down",
                    "chevron-down",
                    "reorder",
                ),
                StudyAction::RemoveChapter => {
                    ("delete-chapter", "Delete chapter", "trash-2", "remove")
                }
                StudyAction::PreviousChapter => (
                    "previous-chapter",
                    "Previous chapter",
                    "chevron-left",
                    "back",
                ),
                StudyAction::NextChapter => {
                    ("next-chapter", "Next chapter", "chevron-right", "forward")
                }
            };
            let bar = bar.downgrade();
            Command::new(
                format!("study:{id}"),
                label,
                icon(glyph),
                move |window, cx| {
                    let _ = bar.update(cx, |bar, cx| bar.act(action, window, cx));
                },
            )
            .keywords(keywords)
        })
        .collect()
}

/// The analysis board's own actions, as its menu and keys offer them.
fn analysis_commands() -> Vec<Command> {
    use crate::pages::analysis::AnalysisPage;
    type Act = fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>);
    let on_board = |act: Act| {
        move |window: &mut Window, cx: &mut App| {
            if let Some(board) = crate::shell::analysis_board(window, cx) {
                board.update(cx, |board, cx| act(board, window, cx));
            }
        }
    };
    vec![
        Command::new(
            "analysis:flip",
            "Flip board",
            icon("refresh-cw"),
            on_board(|board, _, cx| board.flip(cx)),
        )
        .keywords("turn rotate orientation")
        .kbds("F"),
        Command::new(
            "analysis:copy-fen",
            "Copy FEN",
            icon("copy"),
            on_board(|board, window, cx| board.copy_fen(window, cx)),
        )
        .keywords("position clipboard"),
        Command::new(
            "analysis:copy-pgn",
            "Copy PGN",
            icon("file-text"),
            on_board(|board, window, cx| board.copy_pgn(window, cx)),
        )
        .keywords("game moves clipboard"),
        Command::new(
            "analysis:copy-line",
            "Copy line",
            icon("list-tree"),
            on_board(|board, window, cx| board.copy_shown_line(window, cx)),
        )
        .keywords("variation moves pgn clipboard"),
        Command::new(
            "analysis:import",
            "Import a position or game…",
            icon("import"),
            on_board(|board, window, cx| board.open_import(window, cx)),
        )
        .keywords("paste pgn fen link"),
    ]
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
    let mut list = vec![
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
        .keywords("toggle navigation")
        .kbds(if cfg!(target_os = "macos") {
            "⌃ ⌘ S"
        } else {
            "Ctrl B"
        }),
        Command::new(
            "app:shortcuts",
            "Keyboard shortcuts",
            icon("keyboard"),
            crate::shortcuts::open,
        )
        .keywords("keys hotkeys help")
        .kbds(if cfg!(target_os = "macos") {
            "⌘ /"
        } else {
            "Ctrl /"
        }),
    ];
    // Studies' Lichess tab is there only with an account.
    if connected {
        list.push(
            Command::new(
                "studies:lichess",
                "Lichess studies",
                icon("cloud"),
                crate::shell::open_studies_lichess,
            )
            .detail("Studies")
            .keywords("account cloud chapters"),
        );
    }
    list
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
        Mode::Commands => once_each(
            [
                searched("This page", text, page_commands(cx), 8),
                searched("Play", text, play_commands(cx), 12),
                searched("App", text, app_commands(cx), 8),
                searched("Settings", text, settings_commands(cx), 8),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
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
            let pasted = group(Some("Pasted"), pasted(text));
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
            // A pasted position or game link is what was meant: its actions come first.
            once_each(pasted.into_iter().chain(results).collect())
        }
    }
}

/// What a pasted FEN or Lichess game link can open.
fn pasted(text: &str) -> Vec<Command> {
    if let Some((study, chapter)) = crate::game::study_link(text) {
        return vec![Command::new(
            format!("pasted:study:{study}:{chapter}"),
            "Open this study chapter on the analysis board",
            icon("microscope"),
            move |window, cx| crate::pages::players::open_study(&study, &chapter, window, cx),
        )];
    }
    if crate::game::broadcast_link(text).is_some() {
        let text = text.to_string();
        return vec![Command::new(
            "pasted:broadcast",
            "Watch this broadcast",
            icon("radio"),
            move |window, cx| {
                if let Some(link) = crate::game::broadcast_link(&text) {
                    crate::shell::open_broadcast(link, window, cx);
                }
            },
        )];
    }
    if let Some(channel) = kchess_domain::links::tv_link(text) {
        return vec![Command::new(
            format!("pasted:tv:{channel}"),
            "Watch this TV channel",
            icon("tv"),
            move |window, cx| crate::shell::watch_channel(channel.clone(), window, cx),
        )];
    }
    if let Some((system, id)) = kchess_domain::links::tournament_link(text) {
        return vec![Command::new(
            format!("pasted:tournament:{id}"),
            "Open this tournament",
            icon("trophy"),
            move |window, cx| crate::shell::open_event(system, &id, window, cx),
        )];
    }
    if let Some(name) = crate::game::profile_link(text) {
        return vec![Command::new(
            format!("pasted:player:{name}"),
            format!("Open {name}’s profile"),
            icon("user-search"),
            move |window, cx| crate::shell::open_player(&name, window, cx),
        )];
    }
    if let Some(id) = crate::pages::puzzles::puzzle_link(text) {
        let shown = id.clone();
        return vec![
            Command::new(
                format!("pasted:puzzle:{id}"),
                "Solve this puzzle",
                icon("puzzle"),
                move |window, cx| crate::shell::open_puzzle(id.clone(), window, cx),
            )
            .detail(shown),
        ];
    }
    if let Some(link) = crate::game::game_link(text) {
        let id = link.id.clone();
        return vec![
            Command::new(
                format!("pasted:game:{id}"),
                "Open this game on the analysis board",
                icon("microscope"),
                move |window, cx| crate::pages::players::open_linked(&link, window, cx),
            )
            .detail(id),
        ];
    }
    if let Some(pgn) = movetext(text) {
        return vec![Command::new(
            "pasted:moves",
            "Analyse these moves",
            icon("microscope"),
            move |window, cx| {
                let pgn = pgn.clone();
                crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
                    analysis.open_game(&pgn, usize::MAX, None, None, false, cx)
                });
            },
        )];
    }
    let fen = text
        .contains('/')
        .then(|| crate::game::editor_setup(text))
        .flatten();
    let Some(fen) = fen
        .and_then(|setup| crate::game::rules("setupFen", &[setup]).ok())
        .and_then(|fen| fen.as_str().map(str::to_string))
        .filter(|fen| crate::game::position_problem(fen).is_none())
    else {
        return Vec::new();
    };
    let orientation = if fen.split(' ').nth(1) == Some("b") {
        shakmaty::Color::Black
    } else {
        shakmaty::Color::White
    };
    let (analysed, edited) = (fen.clone(), fen);
    vec![
        Command::new(
            "pasted:analyse",
            "Analyse this position",
            icon("microscope"),
            move |window, cx| {
                let fen = analysed.clone();
                crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
                    analysis.load(Some(&fen), cx);
                    true
                });
            },
        ),
        Command::new(
            "pasted:edit",
            "Edit this position",
            icon("pencil-ruler"),
            move |window, cx| {
                let setup = crate::game::setup_from_fen(&edited);
                crate::shell::edit_position(setup, orientation, window, cx)
            },
        ),
    ]
}

/// Paste outside a text field: what the clipboard holds opens as Quick Pick's first suggestion
/// for it would (a game or position on the analysis board, a Lichess link where it belongs).
/// Under a dialog nothing happens; anything else is said in a notice.
pub fn open_pasted(window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) {
        return;
    }
    let text = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    match pasted(text.trim()).into_iter().next() {
        Some(command) => (command.run)(window, cx),
        None => window.push_notification("Copy a PGN, a FEN or a Lichess link to open it", cx),
    }
}

/// New (⌘N): what the page's own New does, asking first where it asks — a game against the
/// computer, over the board or on Lichess, a study, a fresh analysis board. Under a dialog, or
/// on a page with nothing to start, nothing happens.
pub fn new_on_page(window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) {
        return;
    }
    if crate::shell::current_page(cx) == Some(Page::Analysis) {
        crate::shell::new_analysis(window, cx);
        return;
    }
    let commands = page_commands(cx);
    if let Some(at) = new_target(&commands) {
        (commands[at].run)(window, cx);
    }
}

/// Copy outside a text field: the page's game as PGN, or its position as FEN where it has no
/// game (the board editor), as its own Copy command would. Under a dialog, or on a page with
/// nothing to copy, nothing happens.
pub fn copy_shown(window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) {
        return;
    }
    let commands = page_commands(cx);
    if let Some(at) = copy_target(&commands) {
        (commands[at].run)(window, cx);
    }
}

/// Numbered moves (`1. e4 e5 2. Nf3`, with or without PGN headers) the analysis board can play.
fn movetext(text: &str) -> Option<String> {
    let numbered = text.split_whitespace().any(|word| word.starts_with("1."));
    (numbered && crate::game::pgn_analysable(text))
        .then(|| crate::game::tree::TreeNode::from_pgn(text))
        .flatten()
        .filter(|tree| !tree.children.is_empty())
        .map(|_| text.to_string())
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
        return "Search";
    }
    match mode {
        Mode::All => "Search pages and commands (type ? for help)",
        Mode::Commands => "Type a command",
        Mode::Players => "Search players you follow, or type a Lichess username",
        Mode::Settings => "Search settings",
        Mode::Help => "Pick how to search",
    }
}

impl CommandPalette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> CommandPalette {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
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
                            .px(px(12.))
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
                                .gap(px(8.))
                                .px(px(12.))
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
                                                            .font_weight(FontWeight::MEDIUM)
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
                h_flex().gap(px(4.)).child(kbd(keys, &palette)).child(label)
            };
            let footer = h_flex()
                .flex_none()
                .flex_wrap()
                .gap_x(px(14.))
                .gap_y(px(4.))
                .px(px(12.))
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
                        .gap(px(4.))
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
            .h(px(32.))
            .px(px(12.))
            .gap(px(8.))
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
                    .h(px(32.))
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
pub(crate) fn kbd(keys: &str, palette: &Palette) -> impl IntoElement {
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
