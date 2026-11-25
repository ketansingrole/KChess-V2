//! Today's practice: what is due now in each practice that comes back on a schedule, the
//! repertoires' lines, the mistakes from your games and the endgames you missed, counted as
//! each counts them, for Home to list.

use kchess_contracts::types::{Library, RunCard};
use shakmaty::Color;

/// A repertoire with lines due: its study, the study's name and the side drilled.
#[derive(Clone, Debug, PartialEq)]
pub struct DueRepertoire {
    pub study: String,
    pub name: String,
    pub color: Color,
    pub due: usize,
}

/// What is due now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Today {
    /// The study repertoires with lines due, most first.
    pub repertoires: Vec<DueRepertoire>,
    pub mistakes: usize,
    pub endgames: usize,
}

impl Today {
    /// What is due at `now`: of the repertoires drilled from a study still saved, in `library`'s
    /// schedule; of the mistakes `library` keeps; of the endgame drills `missed` (the core's
    /// `runSchedule`) that are the app's drills or your games' endgames.
    pub fn at(library: &Library, missed: &[RunCard], now: f64) -> Today {
        let mut repertoires: Vec<DueRepertoire> = library
            .repertoire_schedule
            .0
            .iter()
            .filter_map(|(key, table)| {
                // A study's repertoire is `studyId:color`; an opening's (`opening:…`) is drilled
                // from the explorer's replies, so its lines are not known ahead.
                let (study, color) = key.rsplit_once(':')?;
                let color = match color {
                    "white" => Color::White,
                    "black" => Color::Black,
                    _ => return None,
                };
                let study = library.studies.iter().find(|saved| saved.id == study)?;
                let due = crate::repertoire::due_in_study(study, table, now);
                (due > 0).then(|| DueRepertoire {
                    study: study.id.clone(),
                    name: study.name.clone(),
                    color,
                    due,
                })
            })
            .collect();
        repertoires.sort_by_key(|repertoire| std::cmp::Reverse(repertoire.due));
        Today {
            repertoires,
            mistakes: library
                .mistakes
                .iter()
                .filter(|drill| drill.due_at_ms() <= now)
                .count(),
            // A drill the app no longer has (its id from an older one) cannot be practised.
            endgames: missed
                .iter()
                .filter(|card| card.card.is_due(now))
                .filter(|card| {
                    card.variant.starts_with("own:")
                        || crate::endgames::drills()
                            .iter()
                            .any(|drill| drill.id == card.variant)
                })
                .count(),
        }
    }

    /// Nothing is due.
    pub fn is_empty(&self) -> bool {
        self.repertoires.is_empty() && self.mistakes == 0 && self.endgames == 0
    }

    /// The repertoire lines due, in every repertoire.
    pub fn lines(&self) -> usize {
        self.repertoires
            .iter()
            .map(|repertoire| repertoire.due)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_domain::training::repetition::{Card, DAY_MS};
    use serde_json::json;

    const NOW: f64 = 1_800_000_000_000.0;

    fn card(due: f64) -> Card {
        Card {
            interval: 0.0,
            ease: 2.5,
            due,
            reps: 0,
            lapses: 1,
        }
    }

    /// A study's repertoire counts its lines due, as Practice → Repertoire does: a line no longer
    /// in the study, a line not due yet, an opening's repertoire and a deleted study's count
    /// for nothing. Mistakes count when due; endgames when their card is, and the drill is one
    /// the app has or your game's.
    #[test]
    fn what_is_due_is_counted_as_each_practice_counts_it() {
        let library: Library = serde_json::from_value(json!({
            "studies": [{
                "id": "s1", "name": "1. e4", "pgn": "1. e4 e5 2. Nf3 (2. Bc4) *", "updatedAt": 1,
                "chapters": [{ "id": "c1", "name": "Main", "pgn": "1. e4 e5 2. Nf3 (2. Bc4) *" }],
            }],
            "games": [],
            "mistakes": [
                { "id": "due", "fen": "8/8/8/8/8/8/1k6/R3K3 w - - 0 1", "solution": ["a1a8"],
                  "judgment": "mistake", "dueAt": NOW - 1.0, "streak": 0, "attempts": 1 },
                { "id": "later", "fen": "8/8/8/8/8/8/1k6/R3K3 w - - 0 1", "solution": ["a1a8"],
                  "judgment": "mistake", "dueAt": NOW + DAY_MS, "streak": 1, "attempts": 1 },
            ],
            "sessions": {},
            "joinedTournaments": [],
            "repertoireMisses": {},
            "repertoireSchedule": {},
        }))
        .expect("a library");
        let mut library = library;
        library.repertoire_schedule.0 = vec![
            (
                "s1:white".into(),
                kchess_domain::keyed::Keyed(vec![
                    ("e2e4 e7e5 g1f3".into(), card(NOW - 1.0)),
                    ("e2e4 e7e5 f1c4".into(), card(NOW + DAY_MS)),
                    ("e2e4 c7c5".into(), card(NOW - 1.0)),
                ]),
            ),
            (
                "s1:black".into(),
                kchess_domain::keyed::Keyed(vec![("e2e4 e7e5 f1c4".into(), card(NOW))]),
            ),
            (
                "opening:Italian Game:white".into(),
                kchess_domain::keyed::Keyed(vec![("e2e4".into(), card(NOW - 1.0))]),
            ),
            (
                "gone:white".into(),
                kchess_domain::keyed::Keyed(vec![("e2e4".into(), card(NOW - 1.0))]),
            ),
        ];
        let missed = [
            RunCard {
                variant: "kr-vs-k".into(),
                card: card(NOW),
            },
            RunCard {
                variant: "kq-vs-k".into(),
                card: card(NOW + DAY_MS),
            },
            RunCard {
                variant: "own:8/8/8/8/8/8/1k6/R3K3 w - - 0 1".into(),
                card: card(NOW),
            },
            RunCard {
                variant: "a-drill-since-removed".into(),
                card: card(NOW),
            },
        ];
        let today = Today::at(&library, &missed, NOW);
        assert_eq!(
            today.repertoires,
            [
                DueRepertoire {
                    study: "s1".into(),
                    name: "1. e4".into(),
                    color: Color::White,
                    due: 1,
                },
                DueRepertoire {
                    study: "s1".into(),
                    name: "1. e4".into(),
                    color: Color::Black,
                    due: 1,
                },
            ]
        );
        assert_eq!((today.lines(), today.mistakes, today.endgames), (2, 1, 2));
        assert!(!today.is_empty());
        assert!(Today::at(&Library::default(), &[], NOW).is_empty());
    }
}
