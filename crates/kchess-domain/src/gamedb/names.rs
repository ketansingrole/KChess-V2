//! The names in a database that are one player's: "Carlsen, Magnus", "Carlsen,M." and "Magnus
//! Carlsen" as different sources write them. A database merges them on request (the core's
//! `gamedb::names`); this finds the likely ones and leaves out any it cannot be sure of.

use std::collections::BTreeMap;

use super::fold_name;

/// A name read as a surname and given names, each folded as a search folds it.
#[derive(Debug, PartialEq)]
struct Parsed {
    surname: String,
    given: Vec<String>,
}

/// "Surname, Given" by its comma, else "Given Surname" by its last word; one word is a surname
/// alone (an account's name).
fn parse(name: &str) -> Option<Parsed> {
    let words = |text: &str| -> Vec<String> {
        fold_name(text)
            .split(' ')
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (surname, given) = match name.split_once(',') {
        Some((surname, given)) => (fold_name(surname), words(given)),
        None => {
            let mut all = words(name);
            let surname = all.pop()?;
            (surname, all)
        }
    };
    (!surname.is_empty()).then_some(Parsed { surname, given })
}

/// The groups of names in `names` (each with its games) that are likely one player's, each with
/// the most played name first and the groups with the most games first. Names group when their
/// surnames match and their first given names agree, an initial matching a whole name ("M" and
/// "Magnus"); an initial that two whole names share ("M" with "Magnus" and "Mads") joins neither.
pub fn same_players(names: &[(String, u64)]) -> Vec<Vec<(String, u64)>> {
    // Each surname with a first initial: whole first names, and the names with an initial.
    type Bucket<'a> = (BTreeMap<&'a str, Vec<usize>>, Vec<usize>);
    let parsed: Vec<Option<Parsed>> = names.iter().map(|(name, _)| parse(name)).collect();
    let mut buckets: BTreeMap<(&str, char), Bucket> = BTreeMap::new();
    for (index, found) in parsed.iter().enumerate() {
        let Some(found) = found else { continue };
        let first = found.given.first().map(String::as_str).unwrap_or("");
        let initial = first.chars().next().unwrap_or(' ');
        let (whole, initials) = buckets.entry((&found.surname, initial)).or_default();
        if first.chars().count() > 1 {
            whole.entry(first).or_default().push(index);
        } else {
            initials.push(index);
        }
    }
    let mut groups: Vec<Vec<(String, u64)>> = Vec::new();
    for (whole, initials) in buckets.into_values() {
        let mut found: Vec<Vec<usize>> = whole.into_values().collect();
        match found.len() {
            0 => found.push(initials),
            1 => found[0].extend(initials),
            _ => {}
        }
        for group in found.into_iter().filter(|group| group.len() > 1) {
            let mut group: Vec<(String, u64)> = group
                .into_iter()
                .map(|index| names[index].clone())
                .collect();
            group.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            groups.push(group);
        }
    }
    let games = |group: &[(String, u64)]| group.iter().map(|(_, games)| games).sum::<u64>();
    groups.sort_by(|a, b| games(b).cmp(&games(a)).then_with(|| a[0].0.cmp(&b[0].0)));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[(&str, u64)]) -> Vec<(String, u64)> {
        names
            .iter()
            .map(|(name, games)| (name.to_string(), *games))
            .collect()
    }

    #[test]
    fn a_name_reads_as_surname_and_given_names() {
        let read = |name| parse(name).map(|found| (found.surname, found.given.join(" ")));
        let both = |surname: &str, given: &str| Some((surname.to_string(), given.to_string()));
        assert_eq!(read("Carlsen, Magnus"), both("carlsen", "magnus"));
        assert_eq!(read("Carlsen,M."), both("carlsen", "m"));
        assert_eq!(read("Magnus Carlsen"), both("carlsen", "magnus"));
        assert_eq!(read("Van Foreest, Jorden"), both("van foreest", "jorden"));
        assert_eq!(read("DrNykterstein"), both("drnykterstein", ""));
        assert_eq!(read(" , "), None);
    }

    #[test]
    fn one_player_s_spellings_group_with_the_most_played_first() {
        let groups = same_players(&named(&[
            ("Carlsen, M", 12),
            ("Carlsen, Magnus", 1203),
            ("Magnus Carlsen", 3),
            ("Carlsen,Magnus", 1),
            ("Nepomniachtchi, Ian", 400),
            ("Ian Nepomniachtchi", 500),
            ("Caruana, Fabiano", 900),
            ("DrNykterstein", 50),
            ("drnykterstein", 2),
        ]));
        assert_eq!(
            groups,
            [
                named(&[
                    ("Carlsen, Magnus", 1203),
                    ("Carlsen, M", 12),
                    ("Magnus Carlsen", 3),
                    ("Carlsen,Magnus", 1)
                ]),
                named(&[("Ian Nepomniachtchi", 500), ("Nepomniachtchi, Ian", 400)]),
                named(&[("DrNykterstein", 50), ("drnykterstein", 2)]),
            ]
        );
    }

    /// Two players who share a surname and an initial stay apart, and the initial joins neither.
    #[test]
    fn a_shared_initial_joins_no_one() {
        let groups = same_players(&named(&[
            ("Polgar, Judit", 300),
            ("Polgar, J.", 5),
            ("Polgar, Zsuzsa", 100),
            ("Polgar, Zsofia", 40),
            ("Polgar,Zsofia", 2),
            ("Carlsen, Henrik", 1),
            ("Carlsen", 7),
        ]));
        assert_eq!(
            groups,
            [
                named(&[("Polgar, Judit", 300), ("Polgar, J.", 5)]),
                named(&[("Polgar, Zsofia", 40), ("Polgar,Zsofia", 2)]),
            ]
        );
        let shared = same_players(&named(&[
            ("Carlsen, Magnus", 9),
            ("Carlsen, Mads", 3),
            ("Carlsen, M", 1),
        ]));
        assert!(shared.is_empty(), "{shared:?}");
    }
}
