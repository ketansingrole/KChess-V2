use crate::theme::AppearanceMode;

use super::navigation::{ActivePane, TAB_OPTIONS};

const ICON_SETTINGS: &str = "assets/icons/settings.svg";
const SCORE_EXACT_LABEL: u32 = 3_000;
const SCORE_ALIAS_OR_KEYWORD: u32 = 2_000;
const SCORE_PARTIAL_LABEL: u32 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandGroup {
    Actions,
    Navigation,
}

impl CommandGroup {
    pub fn heading(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Navigation => "Navigation",
        }
    }

    fn sort_priority(self) -> u8 {
        match self {
            Self::Actions => 0,
            Self::Navigation => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandId {
    SetDarkMode,
    SetLightMode,
    SetSystemMode,
    OpenPlayWithComputer,
    OpenHistory,
    OpenSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAction {
    OpenPane(ActivePane),
    SetAppearance(AppearanceMode),
}

#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    pub id: CommandId,
    pub group: CommandGroup,
    pub label: &'static str,
    pub icon_path: &'static str,
    pub aliases: &'static [&'static str],
    pub keywords: &'static [&'static str],
    pub action: CommandAction,
}

#[derive(Clone, Copy, Debug)]
pub struct CommandMatch {
    pub id: CommandId,
    pub group: CommandGroup,
    pub label: &'static str,
    pub icon_path: &'static str,
    pub action: CommandAction,
    pub score: u32,
}

pub fn match_commands(query: &str) -> Vec<CommandMatch> {
    let normalized_query = query.trim().to_lowercase();
    let tokens: Vec<&str> = normalized_query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect();

    let mut matches = all_command_specs()
        .into_iter()
        .filter_map(|spec| {
            match_score(spec, &normalized_query, &tokens).map(|score| CommandMatch {
                id: spec.id,
                group: spec.group,
                label: spec.label,
                icon_path: spec.icon_path,
                action: spec.action,
                score,
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.group.sort_priority().cmp(&right.group.sort_priority()))
            .then_with(|| left.label.cmp(right.label))
            .then_with(|| left.id.cmp(&right.id))
    });

    matches
}

fn all_command_specs() -> Vec<CommandSpec> {
    let mut specs = vec![
        CommandSpec {
            id: CommandId::SetDarkMode,
            group: CommandGroup::Actions,
            label: "Dark Mode",
            icon_path: ICON_SETTINGS,
            aliases: &["dark", "dark mode"],
            keywords: &["theme", "appearance", "color scheme"],
            action: CommandAction::SetAppearance(AppearanceMode::Dark),
        },
        CommandSpec {
            id: CommandId::SetLightMode,
            group: CommandGroup::Actions,
            label: "Light Mode",
            icon_path: ICON_SETTINGS,
            aliases: &["light", "light mode"],
            keywords: &["theme", "appearance", "color scheme"],
            action: CommandAction::SetAppearance(AppearanceMode::Light),
        },
        CommandSpec {
            id: CommandId::SetSystemMode,
            group: CommandGroup::Actions,
            label: "System Mode",
            icon_path: ICON_SETTINGS,
            aliases: &["system", "system mode", "auto mode"],
            keywords: &["theme", "appearance", "os", "default"],
            action: CommandAction::SetAppearance(AppearanceMode::System),
        },
    ];

    specs.extend(TAB_OPTIONS.into_iter().map(|tab| {
        let (id, aliases) = match tab.pane {
            ActivePane::PlayWithComputer => (
                CommandId::OpenPlayWithComputer,
                &["open play", "play with computer", "go to play"][..],
            ),
            ActivePane::History => (
                CommandId::OpenHistory,
                &["open history", "go to history", "recent games"][..],
            ),
            ActivePane::Settings => (
                CommandId::OpenSettings,
                &["open settings", "go to settings", "preferences"][..],
            ),
        };

        CommandSpec {
            id,
            group: CommandGroup::Navigation,
            label: tab.label,
            icon_path: tab.icon_path,
            aliases: tab.aliases,
            keywords: aliases,
            action: CommandAction::OpenPane(tab.pane),
        }
    }));

    specs
}

fn match_score(spec: CommandSpec, normalized_query: &str, tokens: &[&str]) -> Option<u32> {
    if tokens.is_empty() {
        return Some(0);
    }

    let label = spec.label.to_lowercase();
    let aliases = spec
        .aliases
        .iter()
        .map(|alias| alias.to_lowercase())
        .collect::<Vec<_>>();
    let keywords = spec
        .keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .collect::<Vec<_>>();

    let matches_any_field_for_token = |token: &str| {
        label.contains(token)
            || aliases.iter().any(|alias| alias.contains(token))
            || keywords.iter().any(|keyword| keyword.contains(token))
    };

    if !tokens
        .iter()
        .all(|token| matches_any_field_for_token(token))
    {
        return None;
    }

    if label == normalized_query {
        return Some(SCORE_EXACT_LABEL + normalized_query.len() as u32);
    }

    let all_tokens_match_alias_or_keyword = tokens.iter().all(|token| {
        aliases.iter().any(|alias| alias.contains(token))
            || keywords.iter().any(|keyword| keyword.contains(token))
    });

    if all_tokens_match_alias_or_keyword {
        return Some(SCORE_ALIAS_OR_KEYWORD + tokens.len() as u32);
    }

    Some(SCORE_PARTIAL_LABEL + tokens.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_actions_before_navigation() {
        let matches = match_commands("");
        assert!(matches.len() >= 6);

        let action_count = matches
            .iter()
            .take_while(|command| command.group == CommandGroup::Actions)
            .count();
        assert_eq!(action_count, 3);
        assert!(
            matches
                .iter()
                .skip(action_count)
                .all(|command| command.group == CommandGroup::Navigation)
        );
    }

    #[test]
    fn dark_mode_queries_match_dark_mode_command() {
        for query in ["dark mode", "dark", "theme dark"] {
            let matches = match_commands(query);
            assert_eq!(
                matches.first().map(|command| command.id),
                Some(CommandId::SetDarkMode)
            );
        }
    }

    #[test]
    fn light_and_system_mode_queries_map_correctly() {
        let light = match_commands("light mode");
        assert_eq!(
            light.first().map(|command| command.id),
            Some(CommandId::SetLightMode)
        );

        let system = match_commands("system mode");
        assert_eq!(
            system.first().map(|command| command.id),
            Some(CommandId::SetSystemMode)
        );
    }

    #[test]
    fn navigation_queries_match_navigation_commands() {
        let history = match_commands("history");
        assert!(
            history
                .iter()
                .any(|command| command.id == CommandId::OpenHistory)
        );

        let play = match_commands("play");
        assert!(
            play.iter()
                .any(|command| command.id == CommandId::OpenPlayWithComputer)
        );

        let settings = match_commands("settings");
        assert!(
            settings
                .iter()
                .any(|command| command.id == CommandId::OpenSettings)
        );
    }

    #[test]
    fn ordering_is_deterministic_for_ties() {
        let first = match_commands("mode")
            .into_iter()
            .filter(|command| command.group == CommandGroup::Actions)
            .map(|command| command.label)
            .collect::<Vec<_>>();
        let second = match_commands("mode")
            .into_iter()
            .filter(|command| command.group == CommandGroup::Actions)
            .map(|command| command.label)
            .collect::<Vec<_>>();

        assert_eq!(first, second);
    }
}
