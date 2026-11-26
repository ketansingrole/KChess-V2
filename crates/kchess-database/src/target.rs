//! Choose where games go: a database already on this computer or a study in the library, or a
//! new one named here. Copying games from the Database page and saving the analysis board's game
//! ask it for a database, merging games (`merge`) for a study.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, WindowExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_board::game_db::{Chosen, GameDbHandle};
use kchess_board::widgets;
use kchess_contracts::types::{DatabaseInfo, DatabaseSource, Study};
use kchess_domain::library::MAX_CHAPTERS;

use crate::theme::Palette;
use crate::widgets::Choice;

/// The target that makes a new database or study.
const NEW: &str = "";

/// The databases games may go into: not `except` (the one they come from), and not Your games,
/// which holds only your own.
pub fn targets(databases: &[DatabaseInfo], except: Option<&str>) -> Vec<Choice> {
    std::iter::once(Choice::new(NEW, "New database"))
        .chain(
            databases
                .iter()
                .filter(|info| Some(info.id.as_str()) != except)
                .filter(|info| info.source != DatabaseSource::Own)
                .map(|info| Choice::new(info.id.clone(), info.name.clone())),
        )
        .collect()
}

/// The studies games may go into as a new chapter: a new one, or one with room for a chapter.
pub fn study_targets(studies: &[Study]) -> Vec<Choice> {
    std::iter::once(Choice::new(NEW, "New study"))
        .chain(
            studies
                .iter()
                .filter(|study| study.chapters.len() < MAX_CHAPTERS)
                .map(|study| Choice::new(study.id.clone(), study.name.clone())),
        )
        .collect()
}

/// Where the games go: the target chosen, or a new one's name.
type Pick = Box<dyn Fn(Option<String>, Option<String>, &mut Window, &mut App)>;

/// What the dialog asks and does.
pub struct Ask {
    pub title: &'static str,
    /// The targets offered, a new one first (`targets`, `study_targets`).
    pub choices: Vec<Choice>,
    /// What a target is, naming its list: "Database", "Study".
    pub kind: &'static str,
    /// The target chosen at first, when it is offered; a new one otherwise.
    pub first: Option<String>,
    /// A new target's name at first.
    pub name: String,
    /// The confirm button's label and icon.
    pub confirm: String,
    pub glyph: &'static str,
}

/// Asks where the games go, then gives the choice to `pick`.
pub fn open(
    ask: Ask,
    window: &mut Window,
    cx: &mut App,
    pick: impl Fn(Option<String>, Option<String>, &mut Window, &mut App) + 'static,
) {
    let title = ask.title;
    let dialog = cx.new(|cx| TargetDialog::new(ask, Box::new(pick), window, cx));
    widgets::open_dialog(window, cx, move |dialog_box, _, _| {
        let submit = dialog.clone();
        dialog_box
            .title(title)
            .w(px(420.))
            .child(dialog.clone())
            .on_ok(move |_, window, cx| {
                submit.update(cx, |dialog, cx| dialog.confirm(window, cx));
                false
            })
    });
}

/// Asks where the `chosen` games (`count` of them) of `from` go, then copies them there.
pub fn copy(from: DatabaseInfo, chosen: Chosen, count: u64, window: &mut Window, cx: &mut App) {
    let ask = Ask {
        title: "Copy to a database",
        choices: targets(
            cx.global::<GameDbHandle>().0.read(cx).databases(),
            Some(&from.id),
        ),
        kind: "Database",
        first: None,
        name: from.name.clone(),
        confirm: format!(
            "Copy {}",
            widgets::format_counted(count as f64, "game", "games")
        ),
        glyph: "copy",
    };
    open(ask, window, cx, move |into, name, window, cx| {
        let (from, chosen) = (from.id.clone(), chosen.clone());
        kchess_board::game_db::with_room(None, "Copy anyway", window, cx, move |_, cx| {
            let db = cx.global::<GameDbHandle>().0.clone();
            db.update(cx, |db, cx| {
                db.copy_games(from.clone(), chosen.clone(), into.clone(), name.clone(), cx)
            });
        });
    });
}

pub struct TargetDialog {
    ask: Ask,
    pick: Pick,
    target: Entity<SelectState<Vec<Choice>>>,
    /// The target chosen; `None` for a new one.
    into: Option<String>,
    name: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl TargetDialog {
    fn new(ask: Ask, pick: Pick, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let choices = ask.choices.clone();
        let into = ask
            .first
            .clone()
            .filter(|first| choices.iter().any(|choice| choice.value.as_ref() == first));
        let target = cx.new(|cx| SelectState::new(choices, None, window, cx));
        let first = SharedString::from(into.clone().unwrap_or_default());
        target.update(cx, |state, cx| state.set_selected_value(&first, window, cx));
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name")
                .default_value(ask.name.clone())
        });
        let subscriptions = vec![
            cx.subscribe(&target, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.into = (value.as_ref() != NEW).then(|| value.to_string());
                    cx.notify();
                }
            }),
            cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
        ];
        TargetDialog {
            ask,
            pick,
            target,
            into,
            name,
            _subscriptions: subscriptions,
        }
    }

    fn typed_name(&self, cx: &App) -> Option<String> {
        let name = self.name.read(cx).value().trim().to_string();
        (!name.is_empty()).then_some(name)
    }

    fn ready(&self, cx: &App) -> bool {
        self.into.is_some() || self.typed_name(cx).is_some()
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        let into = self.into.clone();
        let name = into.is_none().then(|| self.typed_name(cx)).flatten();
        window.close_dialog(cx);
        (self.pick)(into, name, window, cx);
    }
}

impl Render for TargetDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        v_flex()
            .gap(px(12.))
            .child(Select::new(&self.target).accessibility_label(self.ask.kind))
            .when(self.into.is_none(), |dialog| {
                dialog.child(Input::new(&self.name).aria_label("Name"))
            })
            .child(widgets::dialog_footer(
                None,
                None,
                [
                    Button::new("target-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx))
                        .into_any_element(),
                    Button::new("target-confirm")
                        .primary()
                        .icon(kchess_ui::icon(self.ask.glyph))
                        .label(self.ask.confirm.clone())
                        .disabled(!self.ready(cx))
                        .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)))
                        .into_any_element(),
                ],
                &palette,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::targets;
    use kchess_contracts::types::{DatabaseInfo, DatabaseSource};

    fn info(id: &str, source: DatabaseSource) -> DatabaseInfo {
        DatabaseInfo {
            id: id.into(),
            name: id.to_uppercase(),
            source,
            games: 1,
            bytes: 1,
            updated_at: 0,
            index_depth: 0,
            attribution: None,
            player: None,
            lichess_rating: None,
        }
    }

    #[test]
    fn games_go_into_a_new_database_or_another_but_never_your_games() {
        let all = [
            info("club", DatabaseSource::Imported),
            info("own", DatabaseSource::Own),
            info("master", DatabaseSource::Master),
        ];
        let names = |except| -> Vec<String> {
            targets(&all, except)
                .into_iter()
                .map(|choice| choice.label.to_string())
                .collect()
        };
        assert_eq!(names(Some("club")), ["New database", "MASTER"]);
        assert_eq!(names(None), ["New database", "CLUB", "MASTER"]);
    }

    #[test]
    fn games_merge_into_a_new_study_or_one_with_room_for_a_chapter() {
        use kchess_contracts::types::{Study, StudyChapter};
        let study = |id: &str, chapters: usize| Study {
            id: id.into(),
            name: id.to_uppercase(),
            pgn: "*".into(),
            chapters: (0..chapters)
                .map(|n| StudyChapter {
                    id: n.to_string(),
                    name: n.to_string(),
                    pgn: "*".into(),
                })
                .collect(),
            updated_at: 0.into(),
            cloud: None,
        };
        let names: Vec<String> = super::study_targets(&[
            study("open", 1),
            study("full", kchess_domain::library::MAX_CHAPTERS),
        ])
        .into_iter()
        .map(|choice| choice.label.to_string())
        .collect();
        assert_eq!(names, ["New study", "OPEN"]);
    }
}
