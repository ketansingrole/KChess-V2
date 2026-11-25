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
    /// `runSchedule`).
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
            endgames: missed.iter().filter(|card| card.card.is_due(now)).count(),
        }
    }

    /// Nothing is due.
    pub fn is_empty(&self) -> bool {
        self.repertoires.is_empty() && self.mistakes == 0 && self.endgames == 0
    }

    /// The repertoire lines due, in every repertoire.
    pub fn lines(&self) -> usize {
        self.repertoires.iter().map(|repertoire| repertoire.due).sum()
    }
}
