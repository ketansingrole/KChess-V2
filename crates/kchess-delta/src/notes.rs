//! The release notes (`apps/gpui/WHATS_NEW.md`): what the app shows after an update, the GitHub
//! release's text and the App Store's What's New. A `## Unreleased` section collects what the next
//! release brings; `cargo xtask release-version` dates it as `## 2026.11.4 (2026-11-20)`.
//!
//! A section opens with a short summary, a paragraph or two, then lists its items under
//! `### Added`, `### Changed` and `### Fixed`: each a `- ` line, one plain sentence, carried on by
//! indented lines. Sections released before the groups list `- **Title**: text` items alone, and
//! read as they were published.

use crate::Version;

/// The notes, from the repository's root.
pub const FILE: &str = "apps/gpui/WHATS_NEW.md";
/// The heading of the section the next release's notes go in.
pub const UNRELEASED: &str = "Unreleased";
/// The longest item, title and text together: a player reads a line at a glance, and skips a
/// longer one however much it holds.
pub const MAX_ITEM: usize = 280;
/// The longest summary: a few sentences on what the release is about.
pub const MAX_SUMMARY: usize = 400;

/// The group an item is listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Something a player can do now that they could not before.
    Added,
    /// Something that works differently, or better.
    Changed,
    /// Something that went wrong and no longer does.
    Fixed,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Added, Kind::Changed, Kind::Fixed];

    /// Its `### ` heading.
    pub fn heading(self) -> &'static str {
        match self {
            Kind::Added => "Added",
            Kind::Changed => "Changed",
            Kind::Fixed => "Fixed",
        }
    }

    fn of(heading: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|kind| kind.heading().eq_ignore_ascii_case(heading.trim()))
    }
}

/// One `## ` section: its heading's version (or `Unreleased`) and date, and its lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section<'a> {
    pub version: &'a str,
    /// `YYYY-MM-DD`.
    pub date: Option<&'a str>,
    pub lines: Vec<&'a str>,
}

/// One item, its Markdown marks dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The group it is listed under; none in a section released before the groups.
    pub kind: Option<Kind>,
    pub title: Option<String>,
    pub text: String,
}

impl Item {
    /// The characters a player reads.
    pub fn length(&self) -> usize {
        self.title.as_deref().unwrap_or_default().chars().count() + self.text.chars().count()
    }
}

/// Every `## ` section of `text`, in its order; anything before the first is left out.
pub fn sections(text: &str) -> Vec<Section<'_>> {
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let (version, date) = match heading.trim().split_once(' ') {
                Some((version, date)) => (version, Some(date.trim().trim_matches(['(', ')']))),
                None => (heading.trim(), None),
            };
            sections.push(Section {
                version,
                date: date.filter(|date| !date.is_empty()),
                lines: Vec::new(),
            });
        } else if let Some(section) = sections.last_mut() {
            section.lines.push(line);
        }
    }
    sections
}

/// The section of release `version`, if `text` has one.
pub fn section<'a>(text: &'a str, version: &str) -> Option<Section<'a>> {
    sections(text)
        .into_iter()
        .find(|section| section.version == version)
}

impl Section<'_> {
    /// The release this section is for; none for `Unreleased`.
    pub fn release(&self) -> Option<Version> {
        Version::parse(self.version)
    }

    /// The section's Markdown, as the GitHub release holds it.
    pub fn body(&self) -> String {
        self.lines.join("\n").trim().to_string()
    }

    /// The summary: the lines before its first item or group, as one paragraph each.
    pub fn summary(&self) -> Option<String> {
        let mut paragraphs: Vec<String> = Vec::new();
        let mut open = false;
        for line in &self.lines {
            let trimmed = line.trim();
            if trimmed.starts_with("- ") || trimmed.starts_with('#') {
                break;
            }
            if trimmed.is_empty() {
                open = false;
            } else if open && let Some(last) = paragraphs.last_mut() {
                last.push(' ');
                last.push_str(&plain(trimmed));
            } else {
                paragraphs.push(plain(trimmed));
                open = true;
            }
        }
        (!paragraphs.is_empty()).then(|| paragraphs.join("\n\n"))
    }

    /// Whether its items are listed under `### Added`, `### Changed` and `### Fixed`.
    pub fn grouped(&self) -> bool {
        self.lines.iter().any(|line| line.starts_with("### "))
    }

    /// The `### ` headings that name no group.
    fn unknown_groups(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|line| line.strip_prefix("### "))
            .filter(|heading| Kind::of(heading).is_none())
            .collect()
    }

    /// The section's items.
    pub fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let mut kind = None;
        for line in &self.lines {
            let trimmed = line.trim();
            if let Some(heading) = line.strip_prefix("### ") {
                kind = Kind::of(heading);
            } else if let Some(item) = trimmed.strip_prefix("- ") {
                items.push(Item {
                    kind,
                    ..item_of(item)
                });
            } else if !trimmed.is_empty()
                && line.starts_with(' ')
                && let Some(item) = items.last_mut()
            {
                item.text.push(' ');
                item.text.push_str(&plain(trimmed));
            }
        }
        items
    }
}

fn item_of(text: &str) -> Item {
    if let Some(rest) = text.strip_prefix("**")
        && let Some((title, text)) = rest.split_once("**")
    {
        let text = text.trim_start_matches([':', '.']).trim();
        return Item {
            kind: None,
            title: Some(plain(title.trim_end_matches(':'))),
            text: plain(text),
        };
    }
    Item {
        kind: None,
        title: None,
        text: plain(text),
    }
}

/// `text` with each Markdown link `[label](target)` as its label.
pub fn unlink(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        let after = after.get(1..).unwrap_or_default();
        let Some((label, target)) = after.split_once("](") else {
            break;
        };
        let Some(end) = target.find(')') else {
            break;
        };
        out.push_str(before);
        out.push_str(label);
        rest = target.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// `text` without Markdown's emphasis and code marks, links as their labels.
pub fn plain(text: &str) -> String {
    unlink(text).replace("**", "").replace('`', "")
}

/// Why `text`'s sections would read badly: an item over [`MAX_ITEM`], a summary over
/// [`MAX_SUMMARY`], a group that is not Added, Changed or Fixed, an item outside the groups of a
/// grouped section; and of the released ones, an undated section, one without items, an item of
/// one before the groups without a title, or versions out of order.
pub fn problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut previous: Option<Version> = None;
    for section in sections(text) {
        if let Some(summary) = section.summary()
            && summary.chars().count() > MAX_SUMMARY
        {
            problems.push(format!(
                "{} has a summary over {MAX_SUMMARY} characters: say in a few sentences what the \
                 release is about, and leave the rest to its items.",
                section.version
            ));
        }
        for heading in section.unknown_groups() {
            problems.push(format!(
                "{} has a group \"{heading}\": list items under Added, Changed or Fixed.",
                section.version
            ));
        }
        let grouped = section.grouped();
        for item in section.items() {
            if grouped && item.kind.is_none() {
                problems.push(format!(
                    "{} has an item outside Added, Changed and Fixed: {}",
                    section.version, item.text
                ));
            }
            let length = item.length();
            if length > MAX_ITEM {
                problems.push(format!(
                    "{} has a {length}-character line, over {MAX_ITEM}: keep the few points a \
                     player notices most. {}: {}",
                    section.version,
                    item.title.as_deref().unwrap_or_default(),
                    item.text
                ));
            }
        }
        let Some(release) = section.release() else {
            continue;
        };
        if section.date.is_none() {
            problems.push(format!("{} has no date.", section.version));
        }
        let items = section.items();
        if items.is_empty() {
            problems.push(format!("{} has no items.", section.version));
        }
        for item in items.iter().filter(|item| !grouped && item.title.is_none()) {
            problems.push(format!(
                "{} has an item without a **Title**: {}",
                section.version, item.text
            ));
        }
        if previous.is_some_and(|newer| newer <= release) {
            problems.push(format!(
                "{} is not older than the section above it.",
                section.version
            ));
        }
        previous = Some(release);
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# What's new\n\nIntro text.\n\n## Unreleased\n\n- **Next**: not yet.\n\n\
        ## 2026.11.4 (2026-11-20)\n\n- **Guess the move**: a drill, see [Practice](https://x).\n  \
        It carries on.\n- A plain `fix`.\n\n## 2026.11.3 (2026-11-04)\n\n- **Older**: before.\n";

    #[test]
    fn the_notes_read_their_sections_and_items() {
        let all = sections(SAMPLE);
        let versions: Vec<_> = all.iter().map(|section| section.version).collect();
        assert_eq!(versions, [UNRELEASED, "2026.11.4", "2026.11.3"]);
        assert_eq!(all[0].release(), None);
        assert_eq!(all[1].date, Some("2026-11-20"));
        assert_eq!(
            all[1].items(),
            [
                Item {
                    kind: None,
                    title: Some("Guess the move".into()),
                    text: "a drill, see Practice. It carries on.".into(),
                },
                Item {
                    kind: None,
                    title: None,
                    text: "A plain fix.".into(),
                },
            ]
        );
        assert_eq!(
            section(SAMPLE, "2026.11.3").map(|section| section.body()),
            Some("- **Older**: before.".to_string())
        );
        assert_eq!(section(SAMPLE, "2026.11.5"), None);
    }

    const GROUPED: &str = "## Unreleased\n\nKChess becomes a database.\nEvery game, offline.\n\n\
        A second paragraph.\n\n### Added\n\n- You can import a PGN file.\n  As large as you like.\n\
        - **Titled**: still reads.\n\n### Fixed\n\n- Voice input no longer freezes.\n";

    #[test]
    fn a_section_reads_its_summary_and_groups() {
        let section = &sections(GROUPED)[0];
        assert!(section.grouped());
        assert_eq!(
            section.summary().as_deref(),
            Some("KChess becomes a database. Every game, offline.\n\nA second paragraph.")
        );
        assert_eq!(
            section.items(),
            [
                Item {
                    kind: Some(Kind::Added),
                    title: None,
                    text: "You can import a PGN file. As large as you like.".into(),
                },
                Item {
                    kind: Some(Kind::Added),
                    title: Some("Titled".into()),
                    text: "still reads.".into(),
                },
                Item {
                    kind: Some(Kind::Fixed),
                    title: None,
                    text: "Voice input no longer freezes.".into(),
                },
            ]
        );
        // A section before the groups has no summary but what precedes its items.
        assert_eq!(sections(SAMPLE)[1].summary(), None);
        assert!(!sections(SAMPLE)[1].grouped());
        // Grouped items need no title once released.
        let released = GROUPED.replace("## Unreleased", "## 2026.11.4 (2026-11-20)");
        assert_eq!(problems(&released), Vec::<String>::new());
    }

    #[test]
    fn a_grouped_section_keeps_to_its_groups() {
        let notes = "## 2026.11.4 (2026-11-20)\n\n- Loose.\n\n### New\n\n- In no group.\n\n\
            ### Fixed\n\n- Fine.\n";
        assert_eq!(
            problems(notes),
            [
                "2026.11.4 has a group \"New\": list items under Added, Changed or Fixed.",
                "2026.11.4 has an item outside Added, Changed and Fixed: Loose.",
                "2026.11.4 has an item outside Added, Changed and Fixed: In no group.",
            ]
        );
        let long = format!(
            "## Unreleased\n\n{}\n\n### Added\n\n- T.\n",
            "a".repeat(MAX_SUMMARY + 1)
        );
        assert!(
            problems(&long)[0].contains("summary over"),
            "{:?}",
            problems(&long)
        );
    }

    #[test]
    fn links_read_as_their_labels() {
        assert_eq!(unlink("see [a](x) and [b](y)."), "see a and b.");
        assert_eq!(unlink("[unclosed](x"), "[unclosed](x");
        assert_eq!(unlink("no [link] here"), "no [link] here");
        // A link's target starts right after its `](`, whatever brackets its text or the rest
        // of the line holds.
        assert_eq!(unlink("[(c)](x) tail"), "(c) tail");
        assert_eq!(unlink("[abcd](x)y) z"), "abcdy) z");
        assert_eq!(unlink("no link [here"), "no link [here");
        assert_eq!(plain("**bold** `code` [l](t)"), "bold code l");
    }

    #[test]
    fn badly_formed_notes_are_named() {
        assert_eq!(
            problems(SAMPLE),
            ["2026.11.4 has an item without a **Title**: A plain fix."]
        );
        let long = format!(
            "## 2026.11.4 (2026-11-20)\n\n- **T**: {}\n",
            "a".repeat(MAX_ITEM)
        );
        assert!(
            problems(&long)[0].contains("over 280"),
            "{:?}",
            problems(&long)
        );
        let fits = format!(
            "## 2026.11.4 (2026-11-20)\n\n- **T**: {}\n",
            "a".repeat(MAX_ITEM - 1)
        );
        assert!(problems(&fits).is_empty());
        assert_eq!(
            problems("## 2026.11.3\n\n## 2026.11.4 (2026-11-20)\n\n- **T**: t\n"),
            [
                "2026.11.3 has no date.",
                "2026.11.3 has no items.",
                "2026.11.4 is not older than the section above it."
            ]
        );
    }
}
