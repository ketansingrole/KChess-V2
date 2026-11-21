//! The puzzle themes as the Train tab, the history and the stats name them, and the opening an
//! opening page asked puzzles of.

use super::*;

pub(crate) use kchess_domain::puzzle::Theme;

pub(super) fn theme_groups() -> Vec<kchess_domain::puzzle::ThemeGroup> {
    kchess_domain::puzzle::theme_groups().to_vec()
}

fn mix() -> Theme {
    Theme {
        key: "mix".into(),
        name: "Healthy mix".into(),
    }
}

pub(super) fn themes() -> Vec<Theme> {
    std::iter::once(mix())
        .chain(theme_groups().into_iter().flat_map(|group| group.themes))
        .collect()
}

pub(super) type ThemeChoices = SearchableVec<SearchableGroup<Choice>>;

/// The mix (and an opening chosen for puzzles), then every theme under its group.
pub(super) fn theme_choices(cx: &App) -> ThemeChoices {
    let mut first = vec![mix()];
    if let Some(opening) = crate::prefs::get::<Option<OpeningAngle>>(cx, OPENING, None) {
        first.push(Theme {
            key: opening.key,
            name: opening.name,
        });
    }
    let choices = |themes: Vec<Theme>| {
        themes
            .into_iter()
            .map(|theme| Choice::new(theme.key, theme.name))
    };
    SearchableVec::new(
        std::iter::once(SearchableGroup::new("").items(choices(first)))
            .chain(
                theme_groups()
                    .into_iter()
                    .map(|group| SearchableGroup::new(group.label).items(choices(group.themes))),
            )
            .collect::<Vec<_>>(),
    )
}

/// An opening chosen for puzzles.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct OpeningAngle {
    pub(super) key: String,
    pub(super) name: String,
}

/// The themes, and after the mix the opening an opening page chose puzzles of.
pub(super) fn theme_items(cx: &App) -> Vec<Theme> {
    let mut list = themes();
    if let Some(opening) = crate::prefs::get::<Option<OpeningAngle>>(cx, OPENING, None) {
        list.insert(
            1,
            Theme {
                key: opening.key,
                name: opening.name,
            },
        );
    }
    list
}

/// The name Lichess gives a theme key; unknown keys split into words.
pub(super) fn theme_name(key: &str, themes: &[Theme]) -> String {
    if let Some(theme) = themes.iter().find(|theme| theme.key == key) {
        return theme.name.clone();
    }
    let mut name = String::new();
    for (index, letter) in key.chars().enumerate() {
        if index == 0 {
            name.extend(letter.to_uppercase());
        } else {
            if letter.is_uppercase() {
                name.push(' ');
            }
            name.push(letter);
        }
    }
    name
}
