//! `db`: the game databases on this computer — list them, import a PGN file, download the master
//! games, a player's games or a month of strong Lichess games, search one's games, print a game,
//! rename and delete.

use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, DatabaseName, GameDbGameId, MasterMonths, PgnFilePath};
use kchess_contracts::types::{
    ExplorerSpeed, GameDbImported, GameDbPhase, GameDbProgress, GameDbQuery, GameDbResult,
    GameDbSide, GameDbSort, GameSite, LichessGames, PlayerGames,
};
use serde_json::json;

use crate::core::{ClientEvent, CoreClient, CoreEvent, absolute};
use crate::print;
use crate::wait::{Next, Wait};

type ClientEvents = tokio::sync::mpsc::UnboundedReceiver<ClientEvent>;

/// A `db` command, as its words ask.
#[derive(Debug, PartialEq)]
pub enum Db {
    List,
    Import {
        path: String,
        into: Option<String>,
        name: Option<String>,
    },
    /// Download the master games' missing months (the newest `months` of them, if given).
    Masters {
        months: Option<u32>,
    },
    /// Download a player's games into their database, replacing those an earlier download
    /// brought.
    Player(PlayerGames),
    /// Download a month of Lichess's rated games, keeping the strong ones.
    Lichess(LichessGames),
    /// List Lichess's months of rated games, and those each database of them holds.
    LichessMonths,
    Games(GameDbQuery),
    Game {
        database: String,
        game: u64,
    },
    Rename {
        database: String,
        name: String,
    },
    Delete {
        database: String,
    },
}

/// The value after flag `flag`, or a message saying it is missing.
fn value<'a>(words: &mut impl Iterator<Item = &'a String>, flag: &str) -> Result<String, String> {
    words
        .next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value."))
}

fn number<T: std::str::FromStr>(text: &str, flag: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{flag} takes a number, not {text}."))
}

/// What `db` and its words ask for.
pub fn parse(words: &[String]) -> Result<Db, String> {
    let (first, rest) = match words.split_first() {
        None => return Ok(Db::List),
        Some((first, rest)) => (first.as_str(), rest),
    };
    let database = |rest: &[String]| -> Result<String, String> {
        rest.first()
            .cloned()
            .ok_or_else(|| format!("db {first} needs a database's id (db list shows them)."))
    };
    match first {
        "list" => Ok(Db::List),
        "import" => {
            let mut words = rest.iter();
            let (mut path, mut into, mut name) = (None, None, None);
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--into" => into = Some(value(&mut words, "--into")?),
                    "--name" => name = Some(value(&mut words, "--name")?),
                    _ if path.is_none() => path = Some(word.clone()),
                    other => return Err(format!("db import does not take {other}.")),
                }
            }
            let path = path.ok_or("db import needs a .pgn, .pgn.zst, .zip or .7z file.")?;
            let path = absolute(std::path::Path::new(&path))
                .to_string_lossy()
                .into_owned();
            Ok(Db::Import { path, into, name })
        }
        "masters" => {
            let mut words = rest.iter();
            let mut months = None;
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--months" => {
                        months = Some(number(&value(&mut words, "--months")?, "--months")?)
                    }
                    other => return Err(format!("db masters does not take {other}.")),
                }
            }
            Ok(Db::Masters { months })
        }
        "player" => {
            let mut words = rest.iter();
            let site = match words.next().map(String::as_str) {
                Some("lichess") => GameSite::Lichess,
                Some("chesscom" | "chess.com") => GameSite::Chesscom,
                Some(other) => {
                    return Err(format!("db player is lichess or chesscom, not {other}."));
                }
                None => return Err("db player needs a site: lichess or chesscom.".into()),
            };
            let username = words
                .next()
                .cloned()
                .ok_or("db player needs the player's username.")?;
            let mut request = PlayerGames {
                site,
                username,
                months: None,
                speeds: None,
                rated: false,
            };
            while let Some(word) = words.next() {
                let flag = word.as_str();
                match flag {
                    "--months" => request.months = Some(number(&value(&mut words, flag)?, flag)?),
                    "--speed" => {
                        let mut speeds = Vec::new();
                        for name in value(&mut words, flag)?.split(',') {
                            speeds.push(ExplorerSpeed::from_key(name.trim()).ok_or_else(|| {
                                format!(
                                    "--speed takes ultraBullet, bullet, blitz, rapid, classical \
                                     or correspondence, not {name}."
                                )
                            })?);
                        }
                        request.speeds = Some(speeds);
                    }
                    "--rated" => request.rated = true,
                    other => return Err(format!("db player does not take {other}.")),
                }
            }
            Ok(Db::Player(request))
        }
        "lichess" => {
            if rest.first().map(String::as_str) == Some("months") {
                return match rest.get(1) {
                    None => Ok(Db::LichessMonths),
                    Some(other) => Err(format!("db lichess months does not take {other}.")),
                };
            }
            let mut words = rest.iter();
            let mut request = LichessGames {
                month: None,
                min_rating: DEFAULT_RATING,
            };
            while let Some(word) = words.next() {
                let flag = word.as_str();
                match flag {
                    "--month" => request.month = Some(value(&mut words, flag)?),
                    "--rating" => request.min_rating = number(&value(&mut words, flag)?, flag)?,
                    other => return Err(format!("db lichess does not take {other}.")),
                }
            }
            Ok(Db::Lichess(request))
        }
        "games" => {
            let mut query = GameDbQuery {
                database: database(rest)?,
                limit: 20,
                ..GameDbQuery::default()
            };
            let mut words = rest.iter().skip(1);
            while let Some(word) = words.next() {
                let flag = word.as_str();
                match flag {
                    "--player" => query.player = Some(value(&mut words, flag)?),
                    "--opponent" => query.opponent = Some(value(&mut words, flag)?),
                    "--event" => query.event = Some(value(&mut words, flag)?),
                    "--eco" => query.eco = Some(value(&mut words, flag)?),
                    "--position" => query.position = Some(value(&mut words, flag)?),
                    "--side" => {
                        query.side = Some(match value(&mut words, flag)?.as_str() {
                            "white" => GameDbSide::White,
                            "black" => GameDbSide::Black,
                            "either" => GameDbSide::Either,
                            other => return Err(format!("--side is white or black, not {other}.")),
                        });
                    }
                    "--result" => {
                        query.result = Some(match value(&mut words, flag)?.as_str() {
                            "white" | "1-0" => GameDbResult::White,
                            "black" | "0-1" => GameDbResult::Black,
                            "draw" | "1/2-1/2" => GameDbResult::Draw,
                            other => {
                                return Err(format!(
                                    "--result is white, draw or black, not {other}."
                                ));
                            }
                        });
                    }
                    "--sort" => {
                        query.sort = Some(match value(&mut words, flag)?.as_str() {
                            "newest" => GameDbSort::Newest,
                            "oldest" => GameDbSort::Oldest,
                            "rating" => GameDbSort::Rating,
                            other => {
                                return Err(format!(
                                    "--sort is newest, oldest or rating, not {other}."
                                ));
                            }
                        });
                    }
                    "--from" => query.year_from = Some(number(&value(&mut words, flag)?, flag)?),
                    "--to" => query.year_to = Some(number(&value(&mut words, flag)?, flag)?),
                    "--min-elo" => query.min_elo = Some(number(&value(&mut words, flag)?, flag)?),
                    "--offset" => query.offset = number(&value(&mut words, flag)?, flag)?,
                    "--limit" => query.limit = number(&value(&mut words, flag)?, flag)?,
                    other => return Err(format!("db games does not take {other}.")),
                }
            }
            Ok(Db::Games(query))
        }
        "game" => {
            let database = database(rest)?;
            let game = rest
                .get(1)
                .ok_or("db game needs the game's id (db games lists them).")?;
            Ok(Db::Game {
                database,
                game: number(game, "db game")?,
            })
        }
        "rename" => {
            let database = database(rest)?;
            let name = rest.get(1..).unwrap_or_default().join(" ");
            if name.trim().is_empty() {
                return Err("db rename needs the new name.".into());
            }
            Ok(Db::Rename { database, name })
        }
        "delete" => Ok(Db::Delete {
            database: database(rest)?,
        }),
        other => Err(format!(
            "Unknown db command {other}: list, import, masters, player, lichess, games, game, \
             rename or delete."
        )),
    }
}

/// The rating both players of a strong Lichess game had at least, unless `--rating` says.
const DEFAULT_RATING: u32 = 2400;

/// An import's progress, as one line.
fn progress_line(progress: &GameDbProgress) -> String {
    let mut line = format!("{:?}", progress.phase);
    if progress.phase == GameDbPhase::Downloading
        && let Some(month) = &progress.message
    {
        line.push_str(&format!(" {month}"));
    }
    line.push_str(&format!(
        ": {} games, {} skipped",
        progress.games, progress.skipped
    ));
    if let Some(total) = progress.total_bytes.filter(|&total| total > 0) {
        line.push_str(&format!(
            ", {:.0}% read",
            progress.bytes as f64 * 100.0 / total as f64
        ));
    }
    line
}

/// Waits for an import (or the master games' download) to end, printing its progress on stderr;
/// Ctrl-C cancels it.
async fn follow(
    core: &CoreClient,
    events: &mut ClientEvents,
    work: impl Future<Output = Result<GameDbImported, String>>,
) -> Result<GameDbImported, String> {
    tokio::pin!(work);
    let mut wait = Wait::new(events);
    let mut cancelled = false;
    loop {
        tokio::select! {
            imported = &mut work => break imported,
            next = wait.next() => match next {
                Next::Event(CoreEvent::GameDbProgress(progress)) => {
                    if !progress.phase.is_terminal() || progress.phase == GameDbPhase::Done {
                        eprintln!("{}", progress_line(&progress));
                    }
                }
                Next::Interrupted if !cancelled => {
                    cancelled = true;
                    eprintln!("Cancelling…");
                    core.request(calls::GameDbCancel {}).await?;
                }
                Next::Closed => return Err("The core closed.".into()),
                _ => {}
            },
        }
    }
}

/// Runs `db`, printing what it answers: JSON, or a game's PGN.
pub async fn run(core: &CoreClient, events: &mut ClientEvents, db: Db) -> Result<(), String> {
    match db {
        Db::List => print(json!(core.request(calls::GameDbStatus {}).await?)),
        Db::Import { path, into, name } => {
            let import = core.request(calls::GameDbImport {
                path: PgnFilePath(path),
                into: into.map(DatabaseId),
                name: name.map(DatabaseName),
            });
            print(json!(follow(core, events, import).await?));
        }
        Db::Masters { months } => {
            let install = core.request(calls::GameDbMasterInstall {
                months: months.map(MasterMonths),
            });
            print(json!(follow(core, events, install).await?));
        }
        Db::Player(request) => {
            let install = core.request(calls::GameDbPlayerInstall { request });
            print(json!(follow(core, events, install).await?));
        }
        Db::Lichess(request) => {
            let install = core.request(calls::GameDbLichessInstall { request });
            print(json!(follow(core, events, install).await?));
        }
        Db::LichessMonths => print(json!(core.request(calls::GameDbLichessMonths {}).await?)),
        Db::Games(query) => {
            // Games are withheld during a live game: the core checks there is none first.
            core.request(calls::ResumeOnline {}).await?;
            print(json!(core.request(calls::GameDbGames { query }).await?));
        }
        Db::Game { database, game } => {
            core.request(calls::ResumeOnline {}).await?;
            let pgn = core
                .request(calls::GameDbGame {
                    database: DatabaseId(database),
                    game: GameDbGameId(game),
                })
                .await?;
            print(pgn.trim_end());
        }
        Db::Rename { database, name } => print(json!(
            core.request(calls::GameDbRename {
                database: DatabaseId(database),
                name: DatabaseName(name),
            })
            .await?
        )),
        Db::Delete { database } => print(json!(
            core.request(calls::GameDbDelete {
                database: DatabaseId(database),
            })
            .await?
        )),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn db_words_make_their_commands() {
        assert_eq!(parse(&[]).unwrap(), Db::List);
        assert_eq!(parse(&words("list")).unwrap(), Db::List);
        let Db::Import { path, into, name } =
            parse(&words("import games.pgn.zst --into abc --name Mine")).unwrap()
        else {
            panic!("an import");
        };
        assert!(std::path::Path::new(&path).is_absolute());
        assert!(path.ends_with("games.pgn.zst"));
        assert_eq!(
            (into.as_deref(), name.as_deref()),
            (Some("abc"), Some("Mine"))
        );
        let Db::Games(query) = parse(&words(
            "games abc --player carlsen --side black --from 2019 --to 2024 --min-elo 2600 \
             --result draw --sort rating --offset 40 --limit 10 --position 8/8/8/8/8/8/8/K1k5",
        ))
        .unwrap() else {
            panic!("a search");
        };
        assert_eq!(query.database, "abc");
        assert_eq!(query.player.as_deref(), Some("carlsen"));
        assert_eq!(query.side, Some(GameDbSide::Black));
        assert_eq!((query.year_from, query.year_to), (Some(2019), Some(2024)));
        assert_eq!(query.min_elo, Some(2600));
        assert_eq!(query.result, Some(GameDbResult::Draw));
        assert_eq!(query.sort, Some(GameDbSort::Rating));
        assert_eq!((query.offset, query.limit), (40, 10));
        assert_eq!(query.position.as_deref(), Some("8/8/8/8/8/8/8/K1k5"));
        assert_eq!(
            parse(&words("game abc 12")).unwrap(),
            Db::Game {
                database: "abc".into(),
                game: 12
            }
        );
        assert_eq!(
            parse(&words("rename abc Club games")).unwrap(),
            Db::Rename {
                database: "abc".into(),
                name: "Club games".into()
            }
        );
        assert_eq!(
            parse(&words("masters")).unwrap(),
            Db::Masters { months: None }
        );
        assert_eq!(
            parse(&words("masters --months 12")).unwrap(),
            Db::Masters { months: Some(12) }
        );
        assert_eq!(
            parse(&words("player lichess DrNykterstein")).unwrap(),
            Db::Player(PlayerGames {
                site: GameSite::Lichess,
                username: "DrNykterstein".into(),
                months: None,
                speeds: None,
                rated: false,
            })
        );
        assert_eq!(
            parse(&words(
                "player chess.com hikaru --months 6 --speed blitz,rapid --rated"
            ))
            .unwrap(),
            Db::Player(PlayerGames {
                site: GameSite::Chesscom,
                username: "hikaru".into(),
                months: Some(6),
                speeds: Some(vec![ExplorerSpeed::Blitz, ExplorerSpeed::Rapid]),
                rated: true,
            })
        );
        assert_eq!(
            parse(&words("lichess")).unwrap(),
            Db::Lichess(LichessGames {
                month: None,
                min_rating: 2400
            })
        );
        assert_eq!(
            parse(&words("lichess --rating 2200 --month 2026-10")).unwrap(),
            Db::Lichess(LichessGames {
                month: Some("2026-10".into()),
                min_rating: 2200
            })
        );
        assert_eq!(parse(&words("lichess months")).unwrap(), Db::LichessMonths);
        assert_eq!(
            parse(&words("delete abc")).unwrap(),
            Db::Delete {
                database: "abc".into()
            }
        );
    }

    #[test]
    fn db_refuses_what_it_cannot_do() {
        for (text, says) in [
            ("import", "needs a .pgn"),
            ("games", "needs a database"),
            ("games abc --limit many", "takes a number"),
            ("games abc --side purple", "white or black"),
            ("games abc --player", "needs a value"),
            ("game abc", "needs the game"),
            ("rename abc", "needs the new name"),
            ("masters --months lots", "takes a number"),
            ("masters abc", "does not take abc"),
            ("player", "needs a site"),
            ("player fics someone", "lichess or chesscom"),
            ("player lichess", "needs the player's username"),
            ("player lichess someone --speed fast", "--speed takes"),
            ("player lichess someone --max 3", "does not take --max"),
            ("lichess --rating high", "takes a number"),
            ("lichess --month", "needs a value"),
            ("lichess months 3", "does not take 3"),
            ("lichess --speed blitz", "does not take --speed"),
            ("frobnicate", "Unknown db command"),
        ] {
            let error = parse(&words(text)).unwrap_err();
            assert!(error.contains(says), "{text}: {error}");
        }
    }
}
