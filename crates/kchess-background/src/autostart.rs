//! The entry that opens KChess when the person logs in, in the place each system reads at login:
//! a launchd agent in `~/Library/LaunchAgents` (macOS), a desktop entry in the XDG autostart
//! folder (Linux), a shortcut in the Startup folder (Windows).
//!
//! Each is a file in a folder of the person's own session, which the person can see and remove
//! with the system's own tools (macOS lists the agent under Login Items, Windows the shortcut under
//! Startup apps). Every function takes the folder, so the tests write to a scratch one.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows_link;

/// What starts at login, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file's name without its extension; on macOS the agent's label too.
    pub name: String,
    /// What the system shows for it.
    pub title: String,
    /// The program started.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// Variables it starts with (a development profile's folder). A Windows shortcut carries
    /// none, so there they are left out.
    pub env: Vec<(String, String)>,
    /// macOS: the app's bundle identifier, so Login Items lists the agent under the app.
    pub bundle_id: Option<String>,
}

/// The extension of an entry's file on this system.
const EXTENSION: &str = if cfg!(target_os = "macos") {
    "plist"
} else if cfg!(windows) {
    "lnk"
} else {
    "desktop"
};

/// The folder this system reads at login, for the person running KChess; `None` when the home
/// folder cannot be found, or on a system with none.
pub fn folder() -> Option<PathBuf> {
    let var = |name: &str| kchess_host::env::var_os(name).filter(|value| !value.is_empty());
    if cfg!(target_os = "macos") {
        var("HOME").map(|home| PathBuf::from(home).join("Library/LaunchAgents"))
    } else if cfg!(windows) {
        var("APPDATA").map(|roaming| {
            PathBuf::from(roaming).join(r"Microsoft\Windows\Start Menu\Programs\Startup")
        })
    } else if cfg!(unix) {
        var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|config| config.is_absolute())
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("autostart"))
    } else {
        None
    }
}

/// The file of the entry named `name` in `folder`.
pub fn file(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.{EXTENSION}"))
}

/// Whether the entry named `name` is in `folder`.
pub fn installed(folder: &Path, name: &str) -> bool {
    file(folder, name).is_file()
}

/// Write `entry` into `folder` (made if missing), replacing the one there: KChess may have moved
/// since it was written, and an update may start it differently.
pub fn install(folder: &Path, entry: &Entry) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let path = file(folder, &entry.name);
    #[cfg(windows)]
    {
        windows_link::write(&path, entry)
    }
    #[cfg(not(windows))]
    {
        let text = if cfg!(target_os = "macos") {
            launch_agent(entry)
        } else {
            desktop_entry(entry)
        };
        // Whole or not at all: a half-written agent or entry would be read at the next login.
        let partial = path.with_extension(format!("{EXTENSION}.partial"));
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, &path)
    }
}

/// Remove the entry named `name` from `folder`; one already gone is no failure.
pub fn remove(folder: &Path, name: &str) -> io::Result<()> {
    match std::fs::remove_file(file(folder, name)) {
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// `text` as XML character data.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The launchd agent that starts `entry` once as the person logs in, in their graphical session
/// only (never over SSH), and does not start it again when it quits.
pub fn launch_agent(entry: &Entry) -> String {
    let string = |text: &str| format!("<string>{}</string>", xml(text));
    let mut arguments = vec![string(&entry.program.to_string_lossy())];
    arguments.extend(entry.args.iter().map(|arg| string(arg)));
    let mut plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t{}\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\t\t{}\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>\n\
         \t<key>ProcessType</key>\n\t<string>Interactive</string>\n",
        string(&entry.name),
        arguments.join("\n\t\t"),
    );
    if let Some(bundle) = &entry.bundle_id {
        plist.push_str(&format!(
            "\t<key>AssociatedBundleIdentifiers</key>\n\t<array>\n\t\t{}\n\t</array>\n",
            string(bundle)
        ));
    }
    if !entry.env.is_empty() {
        plist.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (name, value) in &entry.env {
            plist.push_str(&format!(
                "\t\t<key>{}</key>\n\t\t{}\n",
                xml(name),
                string(value)
            ));
        }
        plist.push_str("\t</dict>\n");
    }
    plist.push_str("</dict>\n</plist>\n");
    plist
}

/// One argument of a desktop entry's `Exec`, quoted where the specification says it must be.
fn exec_arg(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    if !arg.is_empty() && !arg.contains(RESERVED) {
        return arg.to_string();
    }
    let mut quoted = String::from('"');
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// `text` as a desktop entry's string value: no line breaks, and its backslashes escaped.
fn entry_value(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// The XDG autostart desktop entry that starts `entry` as the person logs in.
pub fn desktop_entry(entry: &Entry) -> String {
    let mut words: Vec<String> = Vec::new();
    if !entry.env.is_empty() {
        words.push("env".into());
        words.extend(
            entry
                .env
                .iter()
                .map(|(name, value)| exec_arg(&format!("{name}={value}"))),
        );
    }
    words.push(exec_arg(&entry.program.to_string_lossy()));
    words.extend(entry.args.iter().map(|arg| exec_arg(arg)));
    // `%` starts a field code in Exec; a literal one is doubled.
    let exec = entry_value(&words.join(" ")).replace('%', "%%");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={exec}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        entry_value(&entry.title),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            name: "com.kchess.app".into(),
            title: "KChess".into(),
            program: PathBuf::from("/Applications/KChess & Co.app/Contents/MacOS/KChess"),
            args: vec!["--background".into()],
            env: Vec::new(),
            bundle_id: Some("com.kchess.app".into()),
        }
    }

    /// Installing writes the entry and replaces an older one; removing takes it away, and
    /// removing it again is no failure. Nothing else in the folder is touched.
    #[test]
    fn an_entry_is_written_replaced_and_removed() {
        let scratch = tempfile::tempdir().unwrap();
        let folder = scratch.path().join("autostart");
        let other = scratch.path().join("other.txt");
        std::fs::write(&other, "kept").unwrap();
        assert!(!installed(&folder, "com.kchess.app"));
        let mut first = entry();
        first.program = std::env::current_exe().unwrap();
        install(&folder, &first).unwrap();
        assert!(installed(&folder, "com.kchess.app"));
        let mut moved = first.clone();
        moved.args = vec!["--background".into(), "--again".into()];
        install(&folder, &moved).unwrap();
        assert!(installed(&folder, "com.kchess.app"));
        // One file: the replacement, with nothing half-written left beside it.
        let files: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
        assert_eq!(files.len(), 1, "{files:?}");
        #[cfg(not(windows))]
        {
            let text = std::fs::read_to_string(file(&folder, "com.kchess.app")).unwrap();
            assert!(text.contains("--again"), "{text}");
        }
        remove(&folder, "com.kchess.app").unwrap();
        assert!(!installed(&folder, "com.kchess.app"));
        remove(&folder, "com.kchess.app").unwrap();
        // A folder that was never made has nothing to remove either.
        remove(&scratch.path().join("missing"), "com.kchess.app").unwrap();
        assert_eq!(std::fs::read_to_string(other).unwrap(), "kept");
    }

    /// The agent runs the program with its arguments once at login, in the person's graphical
    /// session, under the app's name in Login Items; text that is XML is escaped.
    #[test]
    fn the_launch_agent_starts_the_app_once_at_login() {
        let mut with_env = entry();
        with_env.env = vec![("KCHESS_USER_DATA_DIR".into(), "/tmp/a <b>".into())];
        let plist = launch_agent(&with_env);
        assert!(plist.contains("<key>Label</key>\n\t<string>com.kchess.app</string>"));
        assert!(plist.contains(
            "<string>/Applications/KChess &amp; Co.app/Contents/MacOS/KChess</string>\n\t\t<string>--background</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(!plist.contains("KeepAlive"));
        assert!(plist.contains("<string>Aqua</string>"));
        assert!(plist.contains("<key>AssociatedBundleIdentifiers</key>"));
        assert!(
            plist
                .contains("<key>KCHESS_USER_DATA_DIR</key>\n\t\t<string>/tmp/a &lt;b&gt;</string>")
        );
        assert!(!launch_agent(&entry()).contains("EnvironmentVariables"));
    }

    /// Exec quotes what the specification reserves, doubles `%`, and escapes backslashes once
    /// for the quoting and once for the file.
    #[test]
    fn the_desktop_entry_quotes_its_command() {
        let plain = Entry {
            program: PathBuf::from("/opt/KChess/kchess"),
            bundle_id: None,
            ..entry()
        };
        let text = desktop_entry(&plain);
        assert!(text.starts_with("[Desktop Entry]\nType=Application\nName=KChess\n"));
        assert!(
            text.contains("\nExec=/opt/KChess/kchess --background\n"),
            "{text}"
        );
        assert!(text.contains("\nX-GNOME-Autostart-enabled=true\n"));
        let odd = Entry {
            program: PathBuf::from("/home/a b/100%/k\"chess"),
            env: vec![("KCHESS_USER_DATA_DIR".into(), "/tmp/$x".into())],
            ..plain
        };
        let text = desktop_entry(&odd);
        assert!(
            text.contains(
                "\nExec=env \"KCHESS_USER_DATA_DIR=/tmp/\\\\$x\" \"/home/a b/100%%/k\\\\\"chess\" --background\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn each_system_has_its_own_folder_and_extension() {
        let path = file(Path::new("start"), "kchess");
        let expected = if cfg!(target_os = "macos") {
            "kchess.plist"
        } else if cfg!(windows) {
            "kchess.lnk"
        } else {
            "kchess.desktop"
        };
        assert_eq!(path, Path::new("start").join(expected));
        if let Some(folder) = folder() {
            assert!(folder.is_absolute(), "{}", folder.display());
            let tail = if cfg!(target_os = "macos") {
                "LaunchAgents"
            } else if cfg!(windows) {
                "Startup"
            } else {
                "autostart"
            };
            assert!(folder.ends_with(tail), "{}", folder.display());
        }
    }
}
