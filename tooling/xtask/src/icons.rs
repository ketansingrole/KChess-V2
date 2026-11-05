//! The icons the app names (`icon("name")` in apps/gpui/src), as SVG files in
//! `apps/gpui/assets/icons`: Microsoft's Fluent UI System Icons (MIT, `THIRD_PARTY_NOTICES.md`),
//! each drawn for its own sizes on that size's pixel grid, and KChess's own drawings in
//! `tooling/xtask/icons` for what Fluent has no icon for (chess pieces, swords, variants), drawn
//! the same way on the 16 px grid. The app keeps its names (`ICONS` maps each to its drawing).
//!
//! `name.svg` is the 16 px drawing and `name@N.svg` the one for N px; GPUI draws an icon with the
//! drawing that fits its size on screen (`vendor/gpui-pre/KCHESS-PATCH.md`). Writing fetches the
//! pinned package; `check` needs no network: every named icon has its files and every file is
//! named.

use std::collections::{BTreeSet, HashMap};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::Digest as _;

use crate::util::{Result, root, say};

/// `@fluentui/svg-icons`, pinned by npm's integrity hash.
const PACKAGE: &str = "https://registry.npmjs.org/@fluentui/svg-icons/-/svg-icons-1.1.345.tgz";
const INTEGRITY: &str = "sha512-Mz5e5BFKtvRN5Yiwrs5JqZRKgd8Rt8S3YiCAuwOWudmm7NA14Y8Ijr8QzMrE6gtNdoUP6/om7ue0lBQJLL57Mw==";

/// The sizes Fluent draws icons at.
const SIZES: [u32; 7] = [12, 16, 20, 24, 28, 32, 48];

/// Each icon the app names, and the Fluent icon drawn for it (`None`: KChess's own drawing).
const ICONS: &[(&str, Option<&str>)] = &[
    ("arrow-left", Some("arrow_left")),
    ("arrow-left-right", Some("arrow_swap")),
    ("arrow-right", Some("arrow_right")),
    ("arrow-up-down", Some("arrow_sort")),
    ("arrow-up-right", Some("arrow_up_right")),
    ("arrow-up-to-line", Some("arrow_upload")),
    ("at-sign", Some("mention")),
    ("atom", None),
    ("audio-lines", Some("sound_wave_circle")),
    ("ban", Some("prohibited")),
    ("bell", Some("alert")),
    ("bell-ring", Some("alert_on")),
    ("book-dashed", Some("book")),
    ("book-marked", Some("bookmark_multiple")),
    ("book-open", Some("book_open")),
    ("book-open-text", Some("book_open")),
    ("book-plus", None),
    ("book-x", Some("book_dismiss")),
    ("bookmark", Some("bookmark")),
    ("bot", Some("bot")),
    ("brick-wall", None),
    ("calendar-days", Some("calendar")),
    ("castle", None),
    ("chart-column", Some("data_bar_vertical")),
    ("chart-line", Some("data_trending")),
    ("chart-no-axes-column", Some("data_histogram")),
    ("chart-pie", Some("data_pie")),
    ("check", Some("checkmark")),
    ("chess-king", None),
    ("chess-knight", None),
    ("chess-pawn", None),
    ("chess-queen", None),
    ("chess-rook", None),
    ("chevron-down", Some("chevron_down")),
    ("chevron-left", Some("chevron_left")),
    ("chevron-right", Some("chevron_right")),
    ("chevron-up", Some("chevron_up")),
    ("chevrons-down", Some("chevron_double_down")),
    ("chevrons-up-down", Some("chevron_up_down")),
    ("circle", Some("circle")),
    ("circle-alert", Some("error_circle")),
    ("circle-check", Some("checkmark_circle")),
    ("circle-dot", Some("record")),
    ("circle-help", Some("question_circle")),
    ("circle-x", Some("dismiss_circle")),
    ("cloud", Some("cloud")),
    ("cloud-download", Some("cloud_arrow_down")),
    ("cloud-upload", Some("cloud_arrow_up")),
    ("coffee", Some("drink_coffee")),
    ("contrast", Some("circle_half_fill")),
    ("copy", Some("copy")),
    ("cpu", Some("developer_board")),
    ("crosshair", Some("target_arrow")),
    ("crown", Some("crown")),
    ("database", Some("database")),
    ("dices", None),
    ("download", Some("arrow_download")),
    ("dumbbell", Some("dumbbell")),
    ("ellipsis", Some("more_horizontal")),
    ("ellipsis-vertical", Some("more_vertical")),
    ("eraser", None),
    ("external-link", Some("open")),
    ("eye", Some("eye")),
    ("eye-off", Some("eye_off")),
    ("fast-forward", Some("fast_forward")),
    ("file-pen-line", Some("document_edit")),
    ("file-text", Some("document_text")),
    ("film", Some("filmstrip")),
    ("flag", Some("flag")),
    ("flame", Some("fire")),
    ("focus", Some("scan")),
    ("folder-open", Some("folder_open")),
    ("gamepad-2", Some("games")),
    ("gauge", Some("gauge")),
    ("globe", Some("globe")),
    ("globe-2", Some("globe")),
    ("graduation-cap", Some("hat_graduation")),
    ("grid-3x3", Some("grid")),
    ("hand", Some("hand_left")),
    ("handshake", Some("handshake")),
    ("hard-drive", Some("hard_drive")),
    ("highlighter", Some("highlight")),
    ("history", Some("history")),
    ("hourglass", Some("hourglass_half")),
    ("house", Some("home")),
    ("image", Some("image")),
    ("import", Some("arrow_import")),
    ("info", Some("info")),
    ("key-round", Some("key")),
    ("keyboard", Some("keyboard")),
    ("layers", Some("stack")),
    ("layout-grid", Some("grid")),
    ("library-big", Some("library")),
    ("lightbulb", Some("lightbulb")),
    ("link", Some("link")),
    ("list-tree", Some("text_bullet_list_tree")),
    ("loader-circle", Some("spinner_ios")),
    ("locate", Some("location_target_square")),
    ("log-in", Some("arrow_enter")),
    ("log-out", Some("arrow_exit")),
    ("mail", Some("mail")),
    ("maximize", Some("full_screen_maximize")),
    ("message-circle", Some("chat")),
    ("message-square-plus", Some("chat_add")),
    ("message-square-text", Some("comment")),
    ("mic", Some("mic")),
    ("mic-off", Some("mic_off")),
    ("microscope", None),
    ("minimize", Some("full_screen_minimize")),
    ("monitor", Some("desktop")),
    ("moon", Some("weather_moon")),
    ("mountain", None),
    ("mouse-pointer-2", Some("cursor")),
    ("mouse-pointer-click", Some("cursor_hover")),
    ("palette", Some("color")),
    ("panel-left", Some("panel_left")),
    ("pause", Some("pause")),
    ("pencil", Some("edit")),
    ("pencil-ruler", Some("pen")),
    ("play", Some("play")),
    ("plus", Some("add")),
    ("puzzle", Some("puzzle_piece")),
    ("rabbit", Some("animal_rabbit")),
    ("radio", Some("live")),
    ("redo-2", Some("arrow_redo")),
    ("refresh-cw", Some("arrow_sync")),
    ("repeat", Some("arrow_repeat_all")),
    ("rocket", Some("rocket")),
    ("rotate-ccw", Some("arrow_counterclockwise")),
    ("rotate-cw", Some("arrow_clockwise")),
    ("save", Some("save")),
    ("scale", Some("scales")),
    ("scissors", Some("cut")),
    ("search", Some("search")),
    ("search-x", Some("search")),
    ("send", Some("send")),
    ("settings-2", Some("settings")),
    ("shield", Some("shield")),
    ("shield-off", Some("shield_dismiss")),
    ("shuffle", Some("arrow_shuffle")),
    ("skip-back", Some("previous")),
    ("skip-forward", Some("next")),
    ("sliders-horizontal", Some("options")),
    ("square", Some("square")),
    ("square-arrow-out-up-right", Some("open")),
    ("sun", Some("weather_sunny")),
    ("sun-moon", Some("brightness_high")),
    ("swatch-book", Some("paint_brush")),
    ("swords", None),
    ("target", Some("target")),
    ("terminal", Some("code")),
    ("timer", Some("timer")),
    ("tower-control", None),
    ("trash-2", Some("delete")),
    ("triangle-alert", Some("warning")),
    ("trophy", Some("trophy")),
    ("turtle", Some("animal_turtle")),
    ("tv", Some("tv")),
    ("undo-2", Some("arrow_undo")),
    ("unlink", Some("link_dismiss")),
    ("upload", Some("arrow_upload")),
    ("user", Some("person")),
    ("user-minus", Some("person_subtract")),
    ("user-plus", Some("person_add")),
    ("user-round", Some("person")),
    ("user-round-plus", Some("person_add")),
    ("user-search", Some("person_search")),
    ("user-x", Some("person_delete")),
    ("users", Some("people")),
    ("users-round", Some("people")),
    ("volume-2", Some("speaker_2")),
    ("wifi-off", Some("cloud_off")),
    ("x", Some("dismiss")),
    ("zap", Some("flash")),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every name written as `icon("…")`.
pub fn named(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find("icon(\"") {
        let before = rest[..at].chars().next_back();
        rest = &rest[at + 6..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        if let Some(end) = rest.find('"') {
            let name = &rest[..end];
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn wanted() -> BTreeSet<String> {
    let mut files = Vec::new();
    sources(&root().join("apps/gpui/src"), &mut files);
    files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|text| named(&text))
        .collect()
}

fn folder() -> PathBuf {
    root().join("apps/gpui/assets/icons")
}

/// KChess's own drawing of `name`.
fn drawing(name: &str) -> PathBuf {
    root().join(format!("tooling/xtask/icons/{name}.svg"))
}

/// The icon a file draws: `name` for `name.svg` and `name@16.svg`.
fn icon_of(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".svg")?;
    Some(stem.split_once('@').map_or(stem, |(name, _)| name))
}

fn present() -> BTreeSet<String> {
    std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| icon_of(&entry.file_name().to_string_lossy()).map(str::to_string))
        .collect()
}

/// `icons --check`: the files match the names in the sources, and each name has its drawing.
pub fn check() -> Result {
    let (wanted, present) = (wanted(), present());
    let missing: Vec<&String> = wanted.difference(&present).collect();
    let unused: Vec<&String> = present.difference(&wanted).collect();
    let unmapped: Vec<&String> = wanted
        .iter()
        .filter(|name| !ICONS.iter().any(|(icon, _)| icon == name))
        .collect();
    let undrawn: Vec<&str> = ICONS
        .iter()
        .filter(|(name, fluent)| fluent.is_none() && !drawing(name).exists())
        .map(|(name, _)| *name)
        .collect();
    if missing.is_empty() && unused.is_empty() && unmapped.is_empty() && undrawn.is_empty() {
        say(&format!("Icons: {} up to date.", wanted.len()));
        return Ok(());
    }
    Err(format!(
        "Icons are out of date (missing {missing:?}, unused {unused:?}, not in ICONS \
         {unmapped:?}, no drawing in tooling/xtask/icons {undrawn:?}). Run `cargo xtask icons`."
    ))
}

/// The Fluent package's regular drawings of `names`, by name and size.
fn fluent(names: &BTreeSet<&str>) -> Result<HashMap<(String, u32), Vec<u8>>> {
    let cache = root().join("target/fluent-svg-icons-1.1.345.tgz");
    if !cache.exists() {
        say("Downloading the Fluent UI System Icons…");
        crate::util::download(PACKAGE, &cache)?;
    }
    let bytes = std::fs::read(&cache).map_err(|cause| cause.to_string())?;
    let digest = base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(&bytes));
    if format!("sha512-{digest}") != INTEGRITY {
        let _ = std::fs::remove_file(&cache);
        return Err("The Fluent icon package does not match its pinned hash.".into());
    }
    let mut drawings = HashMap::new();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice()));
    for entry in archive.entries().map_err(|cause| cause.to_string())? {
        let mut entry = entry.map_err(|cause| cause.to_string())?;
        let path = entry.path().map_err(|cause| cause.to_string())?;
        let Some(file) = path
            .to_string_lossy()
            .strip_prefix("package/icons/")
            .and_then(|file| file.strip_suffix("_regular.svg"))
            .map(str::to_string)
        else {
            continue;
        };
        let Some((name, size)) = file.rsplit_once('_') else {
            continue;
        };
        let Ok(size) = size.parse::<u32>() else {
            continue;
        };
        if !names.contains(name) || !SIZES.contains(&size) {
            continue;
        }
        let mut svg = Vec::new();
        entry
            .read_to_end(&mut svg)
            .map_err(|cause| cause.to_string())?;
        drawings.insert((name.to_string(), size), svg);
    }
    Ok(drawings)
}

/// `icons`: write the files the sources name, and remove the rest.
pub fn write() -> Result {
    let wanted = wanted();
    let icons: Vec<(&str, Option<&str>)> = ICONS
        .iter()
        .filter(|(name, _)| wanted.contains(*name))
        .copied()
        .collect();
    if let Some(name) = wanted
        .iter()
        .find(|name| !icons.iter().any(|(icon, _)| icon == name))
    {
        return Err(format!(
            "No drawing is chosen for the icon {name}: add it to ICONS."
        ));
    }
    let names: BTreeSet<&str> = icons.iter().filter_map(|(_, fluent)| *fluent).collect();
    let drawings = fluent(&names)?;
    std::fs::create_dir_all(folder()).map_err(|cause| cause.to_string())?;
    for entry in std::fs::read_dir(folder()).map_err(|cause| cause.to_string())? {
        let path = entry.map_err(|cause| cause.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "svg") {
            std::fs::remove_file(&path).map_err(|cause| cause.to_string())?;
        }
    }
    let put = |file: String, svg: &[u8]| {
        std::fs::write(folder().join(file), svg).map_err(|cause| cause.to_string())
    };
    let mut files = 0;
    for (name, fluent) in icons {
        let sized: Vec<(u32, Vec<u8>)> = match fluent {
            Some(fluent) => SIZES
                .iter()
                .filter_map(|size| {
                    let svg = drawings.get(&(fluent.to_string(), *size))?;
                    Some((*size, svg.clone()))
                })
                .collect(),
            None => vec![(
                16,
                std::fs::read(drawing(name)).map_err(|cause| format!("{name}: {cause}"))?,
            )],
        };
        let Some((_, base)) = sized.iter().find(|(size, _)| *size == 16) else {
            return Err(format!("{name} has no 16 px drawing."));
        };
        put(format!("{name}.svg"), base)?;
        for (size, svg) in &sized {
            put(format!("{name}@{size}.svg"), svg)?;
        }
        files += 1 + sized.len();
    }
    say(&format!("Icons: wrote {} in {files} files.", wanted.len()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{icon_of, named};

    #[test]
    fn finds_icon_names_written_as_literals() {
        let text =
            r#"icon("circle-dot"); crate::assets::icon("x2"); my_icon("not-this"); icon(name)"#;
        let names: Vec<String> = named(text).into_iter().collect();
        assert_eq!(names, ["circle-dot", "x2"]);
    }

    #[test]
    fn every_size_of_a_drawing_belongs_to_its_icon() {
        assert_eq!(icon_of("circle-dot.svg"), Some("circle-dot"));
        assert_eq!(icon_of("circle-dot@16.svg"), Some("circle-dot"));
        assert_eq!(icon_of("README.md"), None);
    }

    #[test]
    fn the_apps_icons_are_all_present() {
        super::check().unwrap();
    }
}
