//! Famous games to guess through without bringing one: the games of Wikipedia's "List of chess
//! games" that have an article of their own, the moves as that article gives them, grouped by the
//! list's eras. Wikipedia's selection and move text are CC BY-SA 4.0 (THIRD_PARTY_NOTICES.md).

/// A famous game: how it is listed, and its PGN.
pub struct Classic {
    /// The name it is known by, when it has one.
    pub name: Option<&'static str>,
    /// Its players, as the list names them: "Anderssen–Kieseritzky".
    pub players: &'static str,
    pub year: u16,
    /// The list's section it is under.
    pub era: &'static str,
    pub pgn: &'static str,
}

pub const CLASSICS: [Classic; 22] = [
    // Wikipedia: "McDonnell–La Bourdonnais, match 4, game 16".
    Classic {
        name: None,
        players: "McDonnell–La Bourdonnais",
        year: 1834,
        era: "1800s",
        pgn: r#"[Event "Match 4, game 16"]
[Site "London"]
[Date "1834.??.??"]
[White "Alexander McDonnell"]
[Black "Louis-Charles Mahé de La Bourdonnais"]
[Result "0-1"]

1. e4 c5 2. Nf3 Nc6 3. d4 cxd4 4. Nxd4 e5 5. Nxc6 bxc6 6. Bc4 Nf6 7. Bg5 Be7 8. Qe2 d5
9. Bxf6 Bxf6 10. Bb3 O-O 11. O-O a5 12. exd5 cxd5 13. Rd1 d4 14. c4 Qb6 15. Bc2 Bb7 16. Nd2
Rae8 17. Ne4 Bd8 18. c5 Qc6 19. f3 Be7 20. Rac1 f5 21. Qc4+ Kh8 22. Ba4 Qh6 23. Bxe8 fxe4
24. c6 exf3 25. Rc2 Qe3+ 26. Kh1 Bc8 27. Bd7 f2 28. Rf1 d3 29. Rc3 Bxd7 30. cxd7 e4 31. Qc8
Bd8 32. Qc4 Qe1 33. Rc1 d2 34. Qc5 Rg8 35. Rd1 e3 36. Qc3 Qxd1 37. Rxd1 e2 0-1"#,
    },
    // Wikipedia: "Immortal Game".
    Classic {
        name: Some("The Immortal Game"),
        players: "Anderssen–Kieseritzky",
        year: 1851,
        era: "1800s",
        pgn: r#"[Event "Casual game"]
[Site "London"]
[Date "1851.??.??"]
[White "Adolf Anderssen"]
[Black "Lionel Kieseritzky"]
[Result "1-0"]

1. e4 e5 2. f4 exf4 3. Bc4 Qh4+ 4. Kf1 b5 5. Bxb5 Nf6 6. Nf3 Qh6 7. d3 Nh5 8. Nh4 Qg5 9. Nf5
c6 10. g4 Nf6 11. Rg1 cxb5 12. h4 Qg6 13. h5 Qg5 14. Qf3 Ng8 15. Bxf4 Qf6 16. Nc3 Bc5
17. Nd5 Qxb2 18. Bd6 Bxg1 19. e5 Qxa1+ 20. Ke2 Na6 21. Nxg7+ Kd8 22. Qf6+ Nxf6 23. Be7 1-0"#,
    },
    // Wikipedia: "Evergreen Game".
    Classic {
        name: Some("The Evergreen Game"),
        players: "Anderssen–Dufresne",
        year: 1852,
        era: "1800s",
        pgn: r#"[Event "Casual game"]
[Site "Berlin"]
[Date "1852.??.??"]
[White "Adolf Anderssen"]
[Black "Jean Dufresne"]
[Result "1-0"]

1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5 4. b4 Bxb4 5. c3 Ba5 6. d4 exd4 7. O-O d3 8. Qb3 Qf6 9. e5
Qg6 10. Re1 Nge7 11. Ba3 b5 12. Qxb5 Rb8 13. Qa4 Bb6 14. Nbd2 Bb7 15. Ne4 Qf5 16. Bxd3 Qh5
17. Nf6+ gxf6 18. exf6 Rg8 19. Rad1 Qxf3 20. Rxe7+ Nxe7 21. Qxd7+ Kxd7 22. Bf5+ Ke8 23. Bd7+
Kf8 24. Bxe7# 1-0"#,
    },
    // Wikipedia: "Opera Game".
    Classic {
        name: Some("The Opera Game"),
        players: "Morphy–Duke of Brunswick and Count Isouard",
        year: 1858,
        era: "1800s",
        pgn: r#"[Event "Casual game"]
[Site "Paris"]
[Date "1858.??.??"]
[White "Paul Morphy"]
[Black "Duke of Brunswick and Count Isouard"]
[Result "1-0"]

1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 7. Qb3 Qe7 8. Nc3 c6
9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 12. O-O-O Rd8 13. Rxd7 Rxd7 14. Rd1 Qe6 15. Bxd7+
Nxd7 16. Qb8+ Nxb8 17. Rd8 1-0"#,
    },
    // Wikipedia: "Immortal Draw".
    Classic {
        name: Some("The Immortal Draw"),
        players: "Hamppe–Meitner",
        year: 1872,
        era: "1800s",
        pgn: r#"[Event "Casual game"]
[Site "Vienna"]
[Date "1872.??.??"]
[White "Carl Hamppe"]
[Black "Philipp Meitner"]
[Result "1/2-1/2"]

1. e4 e5 2. Nc3 Bc5 3. Na4 Bxf2+ 4. Kxf2 Qh4+ 5. Ke3 Qf4+ 6. Kd3 d5 7. Kc3 Qxe4 8. Kb3 Na6
9. a3 Qxa4+ 10. Kxa4 Nc5+ 11. Kb4 a5+ 12. Kxc5 Ne7 13. Bb5+ Kd8 14. Bc6 b6+ 15. Kb5 Nxc6
16. Kxc6 Bb7+ 17. Kb5 Ba6+ 18. Kc6 Bb7+ 1/2-1/2"#,
    },
    // Wikipedia: "Lasker versus Bauer, Amsterdam, 1889".
    Classic {
        name: None,
        players: "Lasker–Bauer",
        year: 1889,
        era: "1800s",
        pgn: r#"[Event "Amsterdam"]
[Site "Amsterdam"]
[Date "1889.??.??"]
[White "Emanuel Lasker"]
[Black "Johann Hermann Bauer"]
[Result "1-0"]

1. f4 d5 2. e3 Nf6 3. b3 e6 4. Bb2 Be7 5. Bd3 b6 6. Nc3 Bb7 7. Nf3 Nbd7 8. O-O O-O 9. Ne2 c5
10. Ng3 Qc7 11. Ne5 Nxe5 12. Bxe5 Qc6 13. Qe2 a6 14. Nh5 Nxh5 15. Bxh7+ Kxh7 16. Qxh5+ Kg8
17. Bxg7 Kxg7 18. Qg4+ Kh7 19. Rf3 e5 20. Rh3+ Qh6 21. Rxh6+ Kxh6 22. Qd7 Bf6 23. Qxb7 Kg7
24. Rf1 Rab8 25. Qd7 Rfd8 26. Qg4+ Kf8 27. fxe5 Bg7 28. e6 Rb7 29. Qg6 f6 30. Rxf6+ Bxf6
31. Qxf6+ Ke8 32. Qh8+ Ke7 33. Qg7+ Kxe6 34. Qxb7 Rd6 35. Qxa6 d4 36. exd4 cxd4 37. h4 d3
38. Qxd3 1-0"#,
    },
    // Wikipedia: "Rotlewi versus Rubinstein".
    Classic {
        name: None,
        players: "Rotlewi–Rubinstein",
        year: 1907,
        era: "1900–1924",
        pgn: r#"[Event "Łódź"]
[Site "Łódź"]
[Date "1907.??.??"]
[White "Gersz Rotlewi"]
[Black "Akiba Rubinstein"]
[Result "0-1"]

1. d4 d5 2. Nf3 e6 3. e3 c5 4. c4 Nc6 5. Nc3 Nf6 6. dxc5 Bxc5 7. a3 a6 8. b4 Bd6 9. Bb2 O-O
10. Qd2 Qe7 11. Bd3 dxc4 12. Bxc4 b5 13. Bd3 Rd8 14. Qe2 Bb7 15. O-O Ne5 16. Nxe5 Bxe5
17. f4 Bc7 18. e4 Rac8 19. e5 Bb6+ 20. Kh1 Ng4 21. Be4 Qh4 22. g3 Rxc3 23. gxh4 Rd2 24. Qxd2
Bxe4+ 25. Qg2 Rh3 0-1"#,
    },
    // Wikipedia: "Levitsky versus Marshall".
    Classic {
        name: None,
        players: "Levitsky–Marshall",
        year: 1912,
        era: "1900–1924",
        pgn: r#"[Event "Breslau"]
[Site "Breslau"]
[Date "1912.??.??"]
[White "Stepan Levitsky"]
[Black "Frank Marshall"]
[Result "0-1"]

1. d4 e6 2. e4 d5 3. Nc3 c5 4. Nf3 Nc6 5. exd5 exd5 6. Be2 Nf6 7. O-O Be7 8. Bg5 O-O 9. dxc5
Be6 10. Nd4 Bxc5 11. Nxe6 fxe6 12. Bg4 Qd6 13. Bh3 Rae8 14. Qd2 Bb4 15. Bxf6 Rxf6 16. Rad1
Qc5 17. Qe2 Bxc3 18. bxc3 Qxc3 19. Rxd5 Nd4 20. Qh5 Ref8 21. Re5 Rh6 22. Qg5 Rxh3 23. Rc5
Qg3 0-1"#,
    },
    // Wikipedia: "Immortal Zugzwang Game".
    Classic {
        name: Some("The Immortal Zugzwang Game"),
        players: "Sämisch–Nimzowitsch",
        year: 1923,
        era: "1900–1924",
        pgn: r#"[Event "Copenhagen"]
[Site "Copenhagen"]
[Date "1923.??.??"]
[White "Friedrich Sämisch"]
[Black "Aron Nimzowitsch"]
[Result "0-1"]

1. d4 Nf6 2. c4 e6 3. Nf3 b6 4. g3 Bb7 5. Bg2 Be7 6. Nc3 O-O 7. O-O d5 8. Ne5 c6 9. cxd5
cxd5 10. Bf4 a6 11. Rc1 b5 12. Qb3 Nc6 13. Nxc6 Bxc6 14. h3 Qd7 15. Kh2 Nh5 16. Bd2 f5
17. Qd1 b4 18. Nb1 Bb5 19. Rg1 Bd6 20. e4 fxe4 21. Qxh5 Rxf2 22. Qg5 Raf8 23. Kh1 R8f5
24. Qe3 Bd3 25. Rce1 h6 0-1"#,
    },
    // Wikipedia: "Polish Immortal".
    Classic {
        name: Some("The Polish Immortal"),
        players: "Glucksberg–Najdorf",
        year: 1930,
        era: "1925–1949",
        pgn: r#"[Event "Warsaw"]
[Site "Warsaw"]
[Date "1930.??.??"]
[White "Glucksberg"]
[Black "Miguel Najdorf"]
[Result "0-1"]

1. d4 f5 2. c4 Nf6 3. Nc3 e6 4. Nf3 d5 5. e3 c6 6. Bd3 Bd6 7. O-O O-O 8. Ne2 Nbd7 9. Ng5
Bxh2+ 10. Kh1 Ng4 11. f4 Qe8 12. g3 Qh5 13. Kg2 Bg1 14. Nxg1 Qh2+ 15. Kf3 e5 16. dxe5 Ndxe5+
17. fxe5 Nxe5+ 18. Kf4 Ng6+ 19. Kf3 f4 20. exf4 Bg4+ 21. Kxg4 Ne5+ 22. fxe5 h5# 0-1"#,
    },
    // Wikipedia: "Peruvian Immortal".
    Classic {
        name: Some("The Peruvian Immortal"),
        players: "Canal–NN",
        year: 1934,
        era: "1925–1949",
        pgn: r#"[Event "Budapest"]
[Site "Budapest"]
[Date "1934.??.??"]
[White "Esteban Canal"]
[Black "NN"]
[Result "1-0"]

1. e4 d5 2. exd5 Qxd5 3. Nc3 Qa5 4. d4 c6 5. Nf3 Bg4 6. Bf4 e6 7. h3 Bxf3 8. Qxf3 Bb4 9. Be2
Nd7 10. a3 O-O-O 11. axb4 Qxa1+ 12. Kd2 Qxh1 13. Qxc6+ bxc6 14. Ba6# 1-0"#,
    },
    // Wikipedia: "Botvinnik versus Capablanca, AVRO 1938".
    Classic {
        name: None,
        players: "Botvinnik–Capablanca",
        year: 1938,
        era: "1925–1949",
        pgn: r#"[Event "AVRO"]
[Site "Rotterdam"]
[Date "1938.??.??"]
[White "Mikhail Botvinnik"]
[Black "José Raúl Capablanca"]
[Result "1-0"]

1. d4 Nf6 2. c4 e6 3. Nc3 Bb4 4. e3 d5 5. a3 Bxc3+ 6. bxc3 c5 7. cxd5 exd5 8. Bd3 O-O 9. Ne2
b6 10. O-O Ba6 11. Bxa6 Nxa6 12. Bb2 Qd7 13. a4 Rfe8 14. Qd3 c4 15. Qc2 Nb8 16. Rae1 Nc6
17. Ng3 Na5 18. f3 Nb3 19. e4 Qxa4 20. e5 Nd7 21. Qf2 g6 22. f4 f5 23. exf6 Nxf6 24. f5 Rxe1
25. Rxe1 Re8 26. Re6 Rxe6 27. fxe6 Kg7 28. Qf4 Qe8 29. Qe5 Qe7 30. Ba3 Qxa3 31. Nh5+ gxh5
32. Qg5+ Kf8 33. Qxf6+ Kg8 34. e7 Qc1+ 35. Kf2 Qc2+ 36. Kg3 Qd3+ 37. Kh4 Qe4+ 38. Kxh5 Qe2+
39. Kh4 Qe4+ 40. g4 Qe1+ 41. Kh5 1-0"#,
    },
    // Wikipedia: "Uruguayan Immortal".
    Classic {
        name: Some("The Uruguayan Immortal"),
        players: "Molinari–Roux Cabral",
        year: 1943,
        era: "1925–1949",
        pgn: r#"[Event "Uruguayan Championship"]
[Site "Montevideo"]
[Date "1943.??.??"]
[White "B. Molinari"]
[Black "Luis Roux Cabral"]
[Result "0-1"]

1. d4 Nf6 2. Nf3 d5 3. c4 c6 4. Nc3 Nbd7 5. e3 e6 6. Bd3 dxc4 7. Bxc4 b5 8. Bd3 a6 9. O-O c5
10. b3 Bb7 11. Qe2 Qb6 12. Rd1 Be7 13. a4 b4 14. Nb1 Rc8 15. Nbd2 cxd4 16. Nc4 Qa7 17. Nxd4
O-O 18. Bd2 a5 19. Nb5 Qa8 20. Nbd6 Bxg2 21. Nxc8 Rxc8 22. Re1 Bf3 23. Qf1 Qd5 24. e4 Rxc4
25. bxc4 Qh5 26. Bf4 Ng4 27. Be2 Nde5 28. h3 Bc5 29. Bg3 Nxf2 30. Bxf2 Qg5+ 31. Kh2 Qf4+
32. Bg3 Bg1+ 33. Qxg1 Ng4+ 0-1"#,
    },
    // Wikipedia: "Game of the Century (chess)".
    Classic {
        name: Some("The Game of the Century"),
        players: "Byrne–Fischer",
        year: 1956,
        era: "1950s",
        pgn: r#"[Event "Rosenwald Memorial"]
[Site "New York"]
[Date "1956.??.??"]
[White "Donald Byrne"]
[Black "Bobby Fischer"]
[Result "0-1"]

1. Nf3 Nf6 2. c4 g6 3. Nc3 Bg7 4. d4 O-O 5. Bf4 d5 6. Qb3 dxc4 7. Qxc4 c6 8. e4 Nbd7 9. Rd1
Nb6 10. Qc5 Bg4 11. Bg5 Na4 12. Qa3 Nxc3 13. bxc3 Nxe4 14. Bxe7 Qb6 15. Bc4 Nxc3 16. Bc5
Rfe8+ 17. Kf1 Be6 18. Bxb6 Bxc4+ 19. Kg1 Ne2+ 20. Kf1 Nxd4+ 21. Kg1 Ne2+ 22. Kf1 Nc3+
23. Kg1 axb6 24. Qb4 Ra4 25. Qxb6 Nxd1 26. h3 Rxa2 27. Kh2 Nxf2 28. Re1 Rxe1 29. Qd8+ Bf8
30. Nxe1 Bd5 31. Nf3 Ne4 32. Qb8 b5 33. h4 h5 34. Ne5 Kg7 35. Kg1 Bc5+ 36. Kf1 Ng3+ 37. Ke1
Bb4+ 38. Kd1 Bb3+ 39. Kc1 Ne2+ 40. Kb1 Nc3+ 41. Kc1 Rc2 0-1"#,
    },
    // Wikipedia: "Immortal Losing Game".
    Classic {
        name: Some("The Immortal Losing Game"),
        players: "Śliwa–Bronstein",
        year: 1957,
        era: "1950s",
        pgn: r#"[Event "Gotha"]
[Site "Gotha"]
[Date "1957.??.??"]
[White "Bogdan Śliwa"]
[Black "David Bronstein"]
[Result "1-0"]

1. d4 f5 2. g3 g6 3. Bg2 Bg7 4. Nc3 Nf6 5. Bg5 Nc6 6. Qd2 d6 7. h4 e6 8. O-O-O h6 9. Bf4 Bd7
10. e4 fxe4 11. Nxe4 Nd5 12. Ne2 Qe7 13. c4 Nb6 14. c5 dxc5 15. Bxc7 O-O 16. Bd6 Qf7
17. Bxf8 Rxf8 18. dxc5 Nd5 19. f4 Rd8 20. N2c3 Ndb4 21. Nd6 Qf8 22. Nxb7 Nd4 23. Nxd8 Bb5
24. Nxe6 Bd3 25. Bd5 Qf5 26. Nxd4+ Qxd5 27. Nc2 Bxc3 28. bxc3 Qxa2 29. cxb4 1-0"#,
    },
    // Wikipedia: "World Chess Championship 1972".
    Classic {
        name: None,
        players: "Fischer–Spassky, game 6",
        year: 1972,
        era: "1970s",
        pgn: r#"[Event "World Championship, game 6"]
[Site "Reykjavík"]
[Date "1972.??.??"]
[White "Bobby Fischer"]
[Black "Boris Spassky"]
[Result "1-0"]

1. c4 e6 2. Nf3 d5 3. d4 Nf6 4. Nc3 Be7 5. Bg5 O-O 6. e3 h6 7. Bh4 b6 8. cxd5 Nxd5 9. Bxe7
Qxe7 10. Nxd5 exd5 11. Rc1 Be6 12. Qa4 c5 13. Qa3 Rc8 14. Bb5 a6 15. dxc5 bxc5 16. O-O Ra7
17. Be2 Nd7 18. Nd4 Qf8 19. Nxe6 fxe6 20. e4 d4 21. f4 Qe7 22. e5 Rb8 23. Bc4 Kh8 24. Qh3
Nf8 25. b3 a5 26. f5 exf5 27. Rxf5 Nh7 28. Rcf1 Qd8 29. Qg3 Re7 30. h4 Rbb7 31. e6 Rbc7
32. Qe5 Qe8 33. a4 Qd8 34. R1f2 Qe8 35. R2f3 Qd8 36. Bd3 Qe8 37. Qe4 Nf6 38. Rxf6 gxf6
39. Rxf6 Kg8 40. Bc4 Kh8 41. Qf4 1-0"#,
    },
    // Wikipedia: "World Chess Championship 1972".
    Classic {
        name: None,
        players: "Spassky–Fischer, game 11",
        year: 1972,
        era: "1970s",
        pgn: r#"[Event "World Championship, game 11"]
[Site "Reykjavík"]
[Date "1972.??.??"]
[White "Boris Spassky"]
[Black "Bobby Fischer"]
[Result "1-0"]

1. e4 c5 2. Nf3 d6 3. d4 cxd4 4. Nxd4 Nf6 5. Nc3 a6 6. Bg5 e6 7. f4 Qb6 8. Qd2 Qxb2 9. Nb3
Qa3 10. Bxf6 gxf6 11. Be2 h5 12. O-O Nc6 13. Kh1 Bd7 14. Nb1 Qb4 15. Qe3 d5 16. exd5 Ne7
17. c4 Nf5 18. Qd3 h4 19. Bg4 Nd6 20. N1d2 f5 21. a3 Qb6 22. c5 Qb5 23. Qc3 fxg4 24. a4 h3
25. axb5 hxg2+ 26. Kxg2 Rh3 27. Qf6 Nf5 28. c6 Bc8 29. dxe6 fxe6 30. Rfe1 Be7 31. Rxe6 1-0"#,
    },
    // Wikipedia: "World Chess Championship 1972".
    Classic {
        name: None,
        players: "Spassky–Fischer, game 13",
        year: 1972,
        era: "1970s",
        pgn: r#"[Event "World Championship, game 13"]
[Site "Reykjavík"]
[Date "1972.??.??"]
[White "Boris Spassky"]
[Black "Bobby Fischer"]
[Result "0-1"]

1. e4 Nf6 2. e5 Nd5 3. d4 d6 4. Nf3 g6 5. Bc4 Nb6 6. Bb3 Bg7 7. Nbd2 O-O 8. h3 a5 9. a4 dxe5
10. dxe5 Na6 11. O-O Nc5 12. Qe2 Qe8 13. Ne4 Nbxa4 14. Bxa4 Nxa4 15. Re1 Nb6 16. Bd2 a4
17. Bg5 h6 18. Bh4 Bf5 19. g4 Be6 20. Nd4 Bc4 21. Qd2 Qd7 22. Rad1 Rfe8 23. f4 Bd5 24. Nc5
Qc8 25. Qc3 e6 26. Kh2 Nd7 27. Nd3 c5 28. Nb5 Qc6 29. Nd6 Qxd6 30. exd6 Bxc3 31. bxc3 f6
32. g5 hxg5 33. fxg5 f5 34. Bg3 Kf7 35. Ne5+ Nxe5 36. Bxe5 b5 37. Rf1 Rh8 38. Bf6 a3 39. Rf4
a2 40. c4 Bxc4 41. d7 Bd5 42. Kg3 Ra3+ 43. c3 Rha8 44. Rh4 e5 45. Rh7+ Ke6 46. Re7+ Kd6
47. Rxe5 Rxc3+ 48. Kf2 Rc2+ 49. Ke1 Kxd7 50. Rexd5+ Kc6 51. Rd6+ Kb7 52. Rd7+ Ka6 53. R7d2
Rxd2 54. Kxd2 b4 55. h4 Kb5 56. h5 c4 57. Ra1 gxh5 58. g6 h4 59. g7 h3 60. Be7 Rg8 61. Bf8
h2 62. Kc2 Kc6 63. Rd1 b3+ 64. Kc3 h1=Q 65. Rxh1 Kd5 66. Kb2 f4 67. Rd1+ Ke4 68. Rc1 Kd3
69. Rd1+ Ke2 70. Rc1 f3 71. Bc5 Rxg7 72. Rxc4 Rd7 73. Re4+ Kf1 74. Bd4 f2 0-1"#,
    },
    // Wikipedia: "Deep Blue versus Kasparov, 1996, Game 1".
    Classic {
        name: None,
        players: "Deep Blue–Kasparov, 1996 game 1",
        year: 1996,
        era: "1990s",
        pgn: r#"[Event "Deep Blue versus Kasparov, game 1"]
[Site "Philadelphia"]
[Date "1996.??.??"]
[White "Deep Blue"]
[Black "Garry Kasparov"]
[Result "1-0"]

1. e4 c5 2. c3 d5 3. exd5 Qxd5 4. d4 Nf6 5. Nf3 Bg4 6. Be2 e6 7. h3 Bh5 8. O-O Nc6 9. Be3
cxd4 10. cxd4 Bb4 11. a3 Ba5 12. Nc3 Qd6 13. Nb5 Qe7 14. Ne5 Bxe2 15. Qxe2 O-O 16. Rac1 Rac8
17. Bg5 Bb6 18. Bxf6 gxf6 19. Nc4 Rfd8 20. Nxb6 axb6 21. Rfd1 f5 22. Qe3 Qf6 23. d5 Rxd5
24. Rxd5 exd5 25. b3 Kh8 26. Qxb6 Rg8 27. Qc5 d4 28. Nd6 f4 29. Nxb7 Ne5 30. Qd5 f3 31. g3
Nd3 32. Rc7 Re8 33. Nd6 Re1+ 34. Kh2 Nxf2 35. Nxf7+ Kg7 36. Ng5+ Kh6 37. Rxh7+ 1-0"#,
    },
    // Wikipedia: "Deep Blue versus Kasparov, 1997, Game 6".
    Classic {
        name: None,
        players: "Deep Blue–Kasparov, 1997 game 6",
        year: 1997,
        era: "1990s",
        pgn: r#"[Event "Deep Blue versus Kasparov, game 6"]
[Site "New York"]
[Date "1997.??.??"]
[White "Deep Blue"]
[Black "Garry Kasparov"]
[Result "1-0"]

1. e4 c6 2. d4 d5 3. Nc3 dxe4 4. Nxe4 Nd7 5. Ng5 Ngf6 6. Bd3 e6 7. N1f3 h6 8. Nxe6 Qe7
9. O-O fxe6 10. Bg6+ Kd8 11. Bf4 b5 12. a4 Bb7 13. Re1 Nd5 14. Bg3 Kc8 15. axb5 cxb5 16. Qd3
Bc6 17. Bf5 exf5 18. Rxe7 Bxe7 19. c4 1-0"#,
    },
    // Wikipedia: "Kasparov's Immortal".
    Classic {
        name: Some("Kasparov's Immortal"),
        players: "Kasparov–Topalov",
        year: 1999,
        era: "1990s",
        pgn: r#"[Event "Hoogovens"]
[Site "Wijk aan Zee"]
[Date "1999.??.??"]
[White "Garry Kasparov"]
[Black "Veselin Topalov"]
[Result "1-0"]

1. e4 d6 2. d4 Nf6 3. Nc3 g6 4. Be3 Bg7 5. Qd2 c6 6. f3 b5 7. Nge2 Nbd7 8. Bh6 Bxh6 9. Qxh6
Bb7 10. a3 e5 11. O-O-O Qe7 12. Kb1 a6 13. Nc1 O-O-O 14. Nb3 exd4 15. Rxd4 c5 16. Rd1 Nb6
17. g3 Kb8 18. Na5 Ba8 19. Bh3 d5 20. Qf4+ Ka7 21. Rhe1 d4 22. Nd5 Nbxd5 23. exd5 Qd6
24. Rxd4 cxd4 25. Re7+ Kb6 26. Qxd4+ Kxa5 27. b4+ Ka4 28. Qc3 Qxd5 29. Ra7 Bb7 30. Rxb7 Qc4
31. Qxf6 Kxa3 32. Qxa6+ Kxb4 33. c3+ Kxc3 34. Qa1+ Kd2 35. Qb2+ Kd1 36. Bf1 Rd2 37. Rd7 Rxd7
38. Bxc4 bxc4 39. Qxh8 Rd3 40. Qa8 c3 41. Qa4+ Ke1 42. f4 f5 43. Kc1 Rd2 44. Qa7 1-0"#,
    },
    // Wikipedia: "Kasparov versus the World".
    Classic {
        name: Some("Kasparov versus the World"),
        players: "Kasparov–the World",
        year: 1999,
        era: "1990s",
        pgn: r#"[Event "Kasparov versus the World"]
[Site "Internet"]
[Date "1999.??.??"]
[White "Garry Kasparov"]
[Black "The World"]
[Result "1-0"]

1. e4 c5 2. Nf3 d6 3. Bb5+ Bd7 4. Bxd7+ Qxd7 5. c4 Nc6 6. Nc3 Nf6 7. O-O g6 8. d4 cxd4
9. Nxd4 Bg7 10. Nde2 Qe6 11. Nd5 Qxe4 12. Nc7+ Kd7 13. Nxa8 Qxc4 14. Nb6+ axb6 15. Nc3 Ra8
16. a4 Ne4 17. Nxe4 Qxe4 18. Qb3 f5 19. Bg5 Qb4 20. Qf7 Be5 21. h3 Rxa4 22. Rxa4 Qxa4
23. Qxh7 Bxb2 24. Qxg6 Qe4 25. Qf7 Bd4 26. Qb3 f4 27. Qf7 Be5 28. h4 b5 29. h5 Qc4 30. Qf5+
Qe6 31. Qxe6+ Kxe6 32. g3 fxg3 33. fxg3 b4 34. Bf4 Bd4+ 35. Kh1 b3 36. g4 Kd5 37. g5 e6
38. h6 Ne7 39. Rd1 e5 40. Be3 Kc4 41. Bxd4 exd4 42. Kg2 b2 43. Kf3 Kc3 44. h7 Ng6 45. Ke4
Kc2 46. Rh1 d3 47. Kf5 b1=Q 48. Rxb1 Kxb1 49. Kxg6 d2 50. h8=Q d1=Q 51. Qh7 b5 52. Kf6+ Kb2
53. Qh2+ Ka1 54. Qf4 b4 55. Qxb4 Qf3+ 56. Kg7 d5 57. Qd4+ Kb1 58. g6 Qe4 59. Qg1+ Kb2
60. Qf2+ Kc1 61. Kf6 d4 62. g7 1-0"#,
    },
];

/// The eras of the list, in order, each with its games' indices in `CLASSICS`.
pub fn eras() -> Vec<(&'static str, Vec<usize>)> {
    let mut eras: Vec<(&'static str, Vec<usize>)> = Vec::new();
    for (index, classic) in CLASSICS.iter().enumerate() {
        match eras.last_mut() {
            Some((era, games)) if *era == classic.era => games.push(index),
            _ => eras.push((classic.era, vec![index])),
        }
    }
    eras
}

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

    /// Every famous game replays, legally, to the end its article gives: a move copied wrong
    /// stops it short or ends it elsewhere.
    #[test]
    fn every_famous_game_replays_to_its_end() {
        // Plies, and whether the last one mates, as each article's last move and result say.
        let ends = [
            (74, false),
            (45, true),
            (47, true),
            (33, true),
            (36, false),
            (75, false),
            (50, false),
            (46, false),
            (50, false),
            (44, true),
            (27, true),
            (81, false),
            (66, false),
            (82, true),
            (57, false),
            (81, false),
            (61, false),
            (148, false),
            (73, false),
            (37, false),
            (87, false),
            (123, false),
        ];
        for (classic, (plies, mates)) in CLASSICS.iter().zip(ends) {
            let game = super::super::guess::loaded(classic.pgn)
                .unwrap_or_else(|problem| panic!("{}: {problem}", classic.players));
            assert_eq!(game.0.len(), plies, "{}", classic.players);
            let end = view::position_at(&game.1, &game.0, game.0.len()).expect("the end");
            assert_eq!(end.is_checkmate(), mates, "{}", classic.players);
        }
    }

    #[test]
    fn a_famous_game_is_guessed_as_its_winner() {
        // McDonnell lost to La Bourdonnais; Anderssen won the Immortal Game; the Immortal Draw
        // is guessed as White.
        assert_eq!(winner(CLASSICS[0].pgn), shakmaty::Color::Black);
        assert_eq!(winner(CLASSICS[1].pgn), shakmaty::Color::White);
        assert_eq!(CLASSICS[4].name, Some("The Immortal Draw"));
        assert_eq!(winner(CLASSICS[4].pgn), shakmaty::Color::White);
    }

    /// The eras come in the list's order, each once, so the card shows one heading per era.
    #[test]
    fn the_games_are_grouped_by_era_in_order() {
        let eras = super::eras();
        let names: Vec<&str> = eras.iter().map(|(era, _)| *era).collect();
        assert_eq!(
            names,
            ["1800s", "1900–1924", "1925–1949", "1950s", "1970s", "1990s"]
        );
        let years: Vec<u16> = CLASSICS.iter().map(|classic| classic.year).collect();
        assert!(years.is_sorted(), "{years:?}");
        assert_eq!(
            eras.iter().map(|(_, games)| games.len()).sum::<usize>(),
            CLASSICS.len()
        );
    }
}
