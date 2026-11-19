//! Famous games to guess through without bringing one: games every chess player hears of, from the
//! nineteenth century to the computer age. A game's moves are public record.

/// A famous game: its PGN, and how it is named in the list.
pub struct Classic {
    /// The name it is known by.
    pub name: &'static str,
    pub pgn: &'static str,
}

pub const CLASSICS: [Classic; 8] = [
    Classic {
        name: "The Opera Game",
        pgn: r#"[Event "Paris"]
[Date "1858.??.??"]
[White "Paul Morphy"]
[Black "Duke Karl / Count Isouard"]
[Result "1-0"]

1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 7. Qb3 Qe7
8. Nc3 c6 9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 12. O-O-O Rd8 13. Rxd7 Rxd7
14. Rd1 Qe6 15. Bxd7+ Nxd7 16. Qb8+ Nxb8 17. Rd8# 1-0"#,
    },
    Classic {
        name: "The Immortal Game",
        pgn: r#"[Event "London"]
[Date "1851.06.21"]
[White "Adolf Anderssen"]
[Black "Lionel Kieseritzky"]
[Result "1-0"]

1. e4 e5 2. f4 exf4 3. Bc4 Qh4+ 4. Kf1 b5 5. Bxb5 Nf6 6. Nf3 Qh6 7. d3 Nh5
8. Nh4 Qg5 9. Nf5 c6 10. g4 Nf6 11. Rg1 cxb5 12. h4 Qg6 13. h5 Qg5 14. Qf3 Ng8
15. Bxf4 Qf6 16. Nc3 Bc5 17. Nd5 Qxb2 18. Bd6 Bxg1 19. e5 Qxa1+ 20. Ke2 Na6
21. Nxg7+ Kd8 22. Qf6+ Nxf6 23. Be7# 1-0"#,
    },
    Classic {
        name: "The Evergreen Game",
        pgn: r#"[Event "Berlin"]
[Date "1852.??.??"]
[White "Adolf Anderssen"]
[Black "Jean Dufresne"]
[Result "1-0"]

1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5 4. b4 Bxb4 5. c3 Ba5 6. d4 exd4 7. O-O d3
8. Qb3 Qf6 9. e5 Qg6 10. Re1 Nge7 11. Ba3 b5 12. Qxb5 Rb8 13. Qa4 Bb6
14. Nbd2 Bb7 15. Ne4 Qf5 16. Bxd3 Qh5 17. Nf6+ gxf6 18. exf6 Rg8 19. Rad1 Qxf3
20. Rxe7+ Nxe7 21. Qxd7+ Kxd7 22. Bf5+ Ke8 23. Bd7+ Kf8 24. Bxe7# 1-0"#,
    },
    Classic {
        name: "Réti's miniature",
        pgn: r#"[Event "Vienna"]
[Date "1910.??.??"]
[White "Richard Réti"]
[Black "Savielly Tartakower"]
[Result "1-0"]

1. e4 c6 2. d4 d5 3. Nc3 dxe4 4. Nxe4 Nf6 5. Qd3 e5 6. dxe5 Qa5+ 7. Bd2 Qxe5
8. O-O-O Nxe4 9. Qd8+ Kxd8 10. Bg5+ Kc7 11. Bd8# 1-0"#,
    },
    Classic {
        name: "The king hunt",
        pgn: r#"[Event "London"]
[Date "1912.??.??"]
[White "Edward Lasker"]
[Black "George Thomas"]
[Result "1-0"]

1. d4 e6 2. Nf3 f5 3. Nc3 Nf6 4. Bg5 Be7 5. Bxf6 Bxf6 6. e4 fxe4 7. Nxe4 b6
8. Ne5 O-O 9. Bd3 Bb7 10. Qh5 Qe7 11. Qxh7+ Kxh7 12. Nxf6+ Kh6 13. Neg4+ Kg5
14. h4+ Kf4 15. g3+ Kf3 16. Be2+ Kg2 17. Rh2+ Kg1 18. Kd2# 1-0"#,
    },
    Classic {
        name: "The Game of the Century",
        pgn: r#"[Event "Rosenwald Memorial, New York"]
[Date "1956.10.17"]
[White "Donald Byrne"]
[Black "Bobby Fischer"]
[Result "0-1"]

1. Nf3 Nf6 2. c4 g6 3. Nc3 Bg7 4. d4 O-O 5. Bf4 d5 6. Qb3 dxc4 7. Qxc4 c6
8. e4 Nbd7 9. Rd1 Nb6 10. Qc5 Bg4 11. Bg5 Na4 12. Qa3 Nxc3 13. bxc3 Nxe4
14. Bxe7 Qb6 15. Bc4 Nxc3 16. Bc5 Rfe8+ 17. Kf1 Be6 18. Bxb6 Bxc4+ 19. Kg1 Ne2+
20. Kf1 Nxd4+ 21. Kg1 Ne2+ 22. Kf1 Nc3+ 23. Kg1 axb6 24. Qb4 Ra4 25. Qxb6 Nxd1
26. h3 Rxa2 27. Kh2 Nxf2 28. Re1 Rxe1 29. Qd8+ Bf8 30. Nxe1 Bd5 31. Nf3 Ne4
32. Qb8 b5 33. h4 h5 34. Ne5 Kg7 35. Kg1 Bc5+ 36. Kf1 Ng3+ 37. Ke1 Bb4+
38. Kd1 Bb3+ 39. Kc1 Ne2+ 40. Kb1 Nc3+ 41. Kc1 Rc2# 0-1"#,
    },
    Classic {
        name: "Deep Blue's win",
        pgn: r#"[Event "IBM Man-Machine, New York"]
[Date "1997.05.11"]
[White "Deep Blue"]
[Black "Garry Kasparov"]
[Result "1-0"]

1. e4 c6 2. d4 d5 3. Nc3 dxe4 4. Nxe4 Nd7 5. Ng5 Ngf6 6. Bd3 e6 7. N1f3 h6
8. Nxe6 Qe7 9. O-O fxe6 10. Bg6+ Kd8 11. Bf4 b5 12. a4 Bb7 13. Re1 Nd5 14. Bg3 Kc8
15. axb5 cxb5 16. Qd3 Bc6 17. Bf5 exf5 18. Rxe7 Bxe7 19. c4 1-0"#,
    },
    Classic {
        name: "Kasparov's Immortal",
        pgn: r#"[Event "Hoogovens, Wijk aan Zee"]
[Date "1999.01.20"]
[White "Garry Kasparov"]
[Black "Veselin Topalov"]
[Result "1-0"]

1. e4 d6 2. d4 Nf6 3. Nc3 g6 4. Be3 Bg7 5. Qd2 c6 6. f3 b5 7. Nge2 Nbd7 8. Bh6 Bxh6
9. Qxh6 Bb7 10. a3 e5 11. O-O-O Qe7 12. Kb1 a6 13. Nc1 O-O-O 14. Nb3 exd4
15. Rxd4 c5 16. Rd1 Nb6 17. g3 Kb8 18. Na5 Ba8 19. Bh3 d5 20. Qf4+ Ka7 21. Rhe1 d4
22. Nd5 Nbxd5 23. exd5 Qd6 24. Rxd4 cxd4 25. Re7+ Kb6 26. Qxd4+ Kxa5 27. b4+ Ka4
28. Qc3 Qxd5 29. Ra7 Bb7 30. Rxb7 Qc4 31. Qxf6 Kxa3 32. Qxa6+ Kxb4 33. c3+ Kxc3
34. Qa1+ Kd2 35. Qb2+ Kd1 36. Bf1 Rd2 37. Rd7 Rxd7 38. Bxc4 bxc4 39. Qxh8 Rd3
40. Qa8 c3 41. Qa4+ Ke1 42. f4 f5 43. Kc1 Rd2 44. Qa7 1-0"#,
    },
];

/// The side to guess a game as: the winner's, White after a draw.
pub fn winner(pgn: &str) -> shakmaty::Color {
    if pgn.contains("[Result \"0-1\"]") {
        shakmaty::Color::Black
    } else {
        shakmaty::Color::White
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::Position as _;

    use super::{CLASSICS, winner};
    use crate::game::view;

    /// Every famous game replays, legally, to the end it is known for: a move remembered wrong
    /// stops it short or ends it elsewhere.
    #[test]
    fn every_famous_game_replays_to_its_end() {
        // Plies, and whether the last one mates.
        let ends = [
            (33, true),
            (45, true),
            (47, true),
            (21, true),
            (35, true),
            (82, true),
            (37, false),
            (87, false),
        ];
        for (classic, (plies, mates)) in CLASSICS.iter().zip(ends) {
            let game = super::super::guess::loaded(classic.pgn)
                .unwrap_or_else(|problem| panic!("{}: {problem}", classic.name));
            assert_eq!(game.0.len(), plies, "{}", classic.name);
            let end = view::position_at(&game.1, &game.0, game.0.len()).expect("the end");
            assert_eq!(end.is_checkmate(), mates, "{}", classic.name);
        }
    }

    #[test]
    fn a_famous_game_is_guessed_as_its_winner() {
        assert_eq!(winner(CLASSICS[0].pgn), shakmaty::Color::White);
        assert_eq!(winner(CLASSICS[5].pgn), shakmaty::Color::Black);
    }
}
