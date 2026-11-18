//! Settings' categories and the groups the sidebar lists them in, as any page names one to open
//! Settings at (`shell::open_settings`).

use gpui_kit::component::Icon;
use kchess_ui::icon;

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

#[cfg(test)]
mod tests {
    use super::{Section, SectionGroup};

    /// The sidebar lists every category once, under its group, in the groups' order.
    #[test]
    fn each_category_is_listed_once_under_its_group() {
        let listed: Vec<Section> = SectionGroup::ALL
            .into_iter()
            .flat_map(SectionGroup::sections)
            .collect();
        assert_eq!(listed, Section::ALL);
        let headings: Vec<_> = SectionGroup::ALL.map(SectionGroup::heading).into();
        assert_eq!(
            headings,
            ["Interface", "Play", "Analysis", "Lichess", "System"]
        );
        assert_eq!(Section::Voice.label(), "Voice input");
        assert_eq!(Section::Accounts.group(), SectionGroup::Lichess);
    }
}
