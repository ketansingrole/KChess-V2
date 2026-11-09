//! `store-notes <in> <out>`: a release's notes (`apps/gpui/WHATS_NEW.md`'s Markdown, as the
//! GitHub release holds them) as the plain text App Store Connect takes for What's New and
//! TestFlight's What to Test (`app-store-publish.yml`).
//!
//! App Store Connect refuses emoji, pictographic symbols, variation selectors and private-use
//! characters. The notes name the app's own buttons, and some of those are symbols (Settings
//! closes with ✕), so a symbol with a plain look-alike becomes it; any other refused character
//! fails here, and `release-verify` runs the same check before a tag, so a note the store would
//! refuse is rewritten before the release rather than found when it is published.

use crate::util::Result;

/// The most App Store Connect takes.
const LIMIT: usize = 4000;

/// Symbols the notes may name, with the plain character the store takes instead.
const PLAIN: &[(char, char)] = &[('✕', '×'), ('✖', '×'), ('✗', '×')];

/// Whether App Store Connect refuses `c` in What's New and What to Test.
fn refused(c: char) -> bool {
    matches!(c as u32,
        // Miscellaneous Symbols, Dingbats, and Miscellaneous Symbols and Arrows' pictographs.
        0x2600..=0x27BF | 0x2B00..=0x2BFF
        // Emoji and other pictographs.
        | 0x1F000..=0x1FAFF
        // Variation selectors, the emoji joiner and keycap.
        | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF | 0x200D | 0x20E3
        // Private use.
        | 0xE000..=0xF8FF | 0xF0000..=0x10FFFF)
}

/// `notes` as the store's plain text: no Markdown headings, emphasis, code or link targets,
/// symbols as their plain look-alikes. Refused when a character the store refuses remains.
pub fn plain(notes: &str) -> Result<String> {
    let mut lines = Vec::new();
    for line in notes.lines() {
        let line = match line.strip_prefix('#') {
            Some(heading) => heading.trim_start_matches('#').trim_start(),
            None => line,
        };
        lines.push(unlink(line).replace("**", "").replace('`', ""));
    }
    let text: String = lines
        .join("\n")
        .trim()
        .chars()
        .map(|c| {
            PLAIN
                .iter()
                .find(|(symbol, _)| *symbol == c)
                .map_or(c, |(_, plain)| *plain)
        })
        .collect();
    if let Some(c) = text.chars().find(|&c| refused(c)) {
        return Err(format!(
            "The release notes have {c:?} (U+{:04X}), which App Store Connect refuses. Write it \
             in words in apps/gpui/WHATS_NEW.md, or give it a plain look-alike in \
             tooling/xtask/src/store_notes.rs.",
            c as u32
        ));
    }
    let text = if text.is_empty() {
        "Improvements and fixes.".to_string()
    } else {
        text
    };
    Ok(text.chars().take(LIMIT).collect())
}

/// `line` with each Markdown link `[text](target)` as its text.
fn unlink(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else {
            break;
        };
        let target = &after[close + 2..];
        let Some(end) = target.find(')') else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&after[..close]);
        rest = &target[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `store-notes <in> <out>`.
pub fn run(args: &[String]) -> Result {
    let [input, output] = args else {
        return Err("store-notes needs the notes file and the file to write.".into());
    };
    let notes = std::fs::read_to_string(input).map_err(|cause| format!("{input}: {cause}"))?;
    std::fs::write(output, plain(&notes)?).map_err(|cause| format!("{output}: {cause}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-09: 2026.11.7's notes said "Close it with ✕ or Escape" (the Settings window's
    /// button), and App Store Connect refused them, so the published release never reached
    /// TestFlight or App Review.
    #[test]
    fn a_close_button_symbol_becomes_a_plain_cross() {
        let notes = "- **Settings in a window of its own**: Close it with ✕ or Escape.";
        assert_eq!(
            plain(notes).unwrap(),
            "- Settings in a window of its own: Close it with × or Escape."
        );
    }

    /// A link's target starts right after its `](`, whatever brackets its text or the rest of
    /// the line holds.
    #[test]
    fn a_link_keeps_its_text_and_the_rest_of_the_line() {
        assert_eq!(unlink("[(c)](x) tail"), "(c) tail");
        assert_eq!(unlink("[abcd](x)y) z"), "abcdy) z");
        assert_eq!(unlink("no link [here"), "no link [here");
    }

    #[test]
    fn the_command_writes_the_plain_notes() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("notes.md");
        let output = dir.path().join("notes.txt");
        std::fs::write(&input, "- **Settings**: close it with ✕.\n").unwrap();
        let args = [input, output.clone()].map(|path| path.display().to_string());
        run(&args).unwrap();
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "- Settings: close it with ×."
        );
        assert!(run(&args[..1]).is_err());
    }

    #[test]
    fn markdown_becomes_plain_text() {
        let notes = "## 2026.11.7 (2026-11-09)\n\n- **Fixes**: see [the site](https://example.com) \
                     and `Settings → Updates`.\n";
        assert_eq!(
            plain(notes).unwrap(),
            "2026.11.7 (2026-11-09)\n\n- Fixes: see the site and Settings → Updates."
        );
    }

    #[test]
    fn a_refused_character_is_named() {
        for notes in [
            "Now with 🎉",
            "Variation\u{FE0F}",
            "Private \u{E000}",
            "Star ★",
        ] {
            let error = plain(notes).unwrap_err();
            assert!(
                error.contains("App Store Connect refuses"),
                "{notes}: {error}"
            );
        }
        assert!(plain("Dashes – quotes “” and arrows → stay").is_ok());
    }

    #[test]
    fn empty_notes_say_something_and_long_ones_are_cut() {
        assert_eq!(plain("  \n").unwrap(), "Improvements and fixes.");
        assert_eq!(plain(&"é".repeat(5000)).unwrap().chars().count(), LIMIT);
    }

    /// Every release's notes, published or not, reach the store.
    #[test]
    fn every_section_of_whats_new_goes_to_the_store() {
        let notes =
            std::fs::read_to_string(crate::util::root().join("apps/gpui/WHATS_NEW.md")).unwrap();
        for section in notes.split("\n## ").skip(1) {
            let heading = section.lines().next().unwrap_or_default();
            if let Err(error) = plain(section) {
                panic!("## {heading}: {error}");
            }
        }
    }
}
