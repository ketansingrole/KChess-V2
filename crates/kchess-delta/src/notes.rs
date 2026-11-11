//! The release notes (`apps/gpui/WHATS_NEW.md`): what the app shows after an update, the GitHub
//! release's text and the App Store's What's New. A `## Unreleased` section collects what the next
//! release brings; `cargo xtask release-version` dates it as `## 2026.11.4 (2026-11-20)`. Each item
//! is a `- **Title**: text` line, carried on by indented lines.

use crate::Version;

/// The notes, from the repository's root.
pub const FILE: &str = "apps/gpui/WHATS_NEW.md";
/// The heading of the section the next release's notes go in.
pub const UNRELEASED: &str = "Unreleased";
/// The longest item, title and text together: a player reads a line at a glance, and skips a
/// longer one however much it holds.
pub const MAX_ITEM: usize = 280;

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

    /// The section's items.
    pub fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        for line in &self.lines {
            let trimmed = line.trim();
            if let Some(item) = trimmed.strip_prefix("- ") {
                items.push(item_of(item));
            } else if !trimmed.is_empty()
                && line.starts_with(' ')
                && let Some(item) = items.last_mut()
            {
                // A wrapped item carries on, indented.
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
            title: Some(plain(title.trim_end_matches(':'))),
            text: plain(text),
        };
    }
    Item {
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

/// Why `text`'s released sections would read badly: an item over [`MAX_ITEM`], an undated
/// section, one without items, an item without a title, or versions out of order.
pub fn problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut previous: Option<Version> = None;
    for section in sections(text) {
        for item in section.items() {
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
        for item in items.iter().filter(|item| item.title.is_none()) {
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
                    title: Some("Guess the move".into()),
                    text: "a drill, see Practice. It carries on.".into(),
                },
                Item {
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
