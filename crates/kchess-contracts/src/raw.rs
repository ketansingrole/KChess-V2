//! Declarations that ts-rs cannot derive, written out as TypeScript: the `components[...]`
//! and `paths[...]` indexed aliases, the generic `AppData<S>`, `typeof`/`keyof` aliases and the
//! numeric `as const` arrays of `types.ts`. Each one is a closest faithful form of the original.

use crate::generate::Item;
use crate::numbers;

/// The numeric `as const` arrays and their union aliases, in `types.ts` order.
const CORRESPONDENCE_DAYS: &[f64] = &[1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
const ARENA_CLOCK_MINUTES: &[f64] = &[
    0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 15.0, 20.0, 25.0,
    30.0, 40.0, 50.0, 60.0,
];
const ARENA_INCREMENTS: &[f64] = &[
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0,
];
const ARENA_DURATIONS: &[f64] = &[
    20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0,
    150.0, 180.0, 210.0, 240.0, 270.0, 300.0, 330.0, 360.0, 420.0, 480.0, 540.0, 600.0, 720.0,
];
const ARENA_WAIT_MINUTES: &[f64] = &[1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0];
const EXPLORER_RATINGS: &[f64] = &[
    400.0, 1000.0, 1200.0, 1400.0, 1600.0, 1800.0, 2000.0, 2200.0, 2500.0,
];

/// The declarations `types.ts` owns that ts-rs does not write (see the module docs).
pub fn types_items() -> Vec<Item> {
    let raw = |name: &str, text: &str| Item::Raw {
        name: name.into(),
        text: text.into(),
    };
    let mut out = vec![
        Item::Const {
            name: "CORRESPONDENCE_DAYS".into(),
            values: numbers(CORRESPONDENCE_DAYS),
            suffix: String::new(),
        },
        raw(
            "CorrespondenceDays",
            "export type CorrespondenceDays = (typeof CORRESPONDENCE_DAYS)[number];",
        ),
        Item::Const {
            name: "ARENA_CLOCK_MINUTES".into(),
            values: numbers(ARENA_CLOCK_MINUTES),
            suffix: String::new(),
        },
        Item::Const {
            name: "ARENA_INCREMENTS".into(),
            values: numbers(ARENA_INCREMENTS),
            suffix: String::new(),
        },
        Item::Const {
            name: "ARENA_DURATIONS".into(),
            values: numbers(ARENA_DURATIONS),
            suffix: String::new(),
        },
        Item::Const {
            name: "ARENA_WAIT_MINUTES".into(),
            values: numbers(ARENA_WAIT_MINUTES),
            suffix: String::new(),
        },
        Item::Const {
            name: "EXPLORER_RATINGS".into(),
            values: numbers(EXPLORER_RATINGS),
            suffix: String::new(),
        },
    ];
    out.extend([
        raw(
            "LichessUser",
            "export type LichessUser = components['schemas']['UserExtended'];",
        ),
        raw(
            "LichessRatingHistory",
            "export type LichessRatingHistory = components['schemas']['RatingHistory'];",
        ),
        raw(
            "LichessNowPlaying",
            "export type LichessNowPlaying =\n  paths['/api/account/playing']['get']['responses'][200]['content']['application/json'];",
        ),
        raw("LichessGameFullEvent", "export type LichessGameFullEvent = components['schemas']['GameFullEvent'];"),
        raw("LichessGameStateEvent", "export type LichessGameStateEvent = components['schemas']['GameStateEvent'];"),
        raw("LichessGameStartEvent", "export type LichessGameStartEvent = components['schemas']['GameStartEvent'];"),
        raw("LichessGameFinishEvent", "export type LichessGameFinishEvent = components['schemas']['GameFinishEvent'];"),
        raw("LichessChallengeEvent", "export type LichessChallengeEvent = components['schemas']['ChallengeEvent'];"),
        raw(
            "LichessChallengeCanceledEvent",
            "export type LichessChallengeCanceledEvent = components['schemas']['ChallengeCanceledEvent'];",
        ),
        raw(
            "LichessChallengeDeclinedEvent",
            "export type LichessChallengeDeclinedEvent = components['schemas']['ChallengeDeclinedEvent'];",
        ),
        raw("LichessChatLineEvent", "export type LichessChatLineEvent = components['schemas']['ChatLineEvent'];"),
        raw(
            "LichessOpponentGoneEvent",
            "export type LichessOpponentGoneEvent = components['schemas']['OpponentGoneEvent'];",
        ),
        raw(
            "OnlineEvent",
            "export type OnlineEvent =\n  | LichessGameFullEvent\n  | LichessGameStartEvent\n  | LichessGameFinishEvent\n  | LichessGameStateEvent\n  | LichessChallengeEvent\n  | LichessChallengeCanceledEvent\n  | LichessChallengeDeclinedEvent\n  | LichessChatLineEvent\n  | LichessOpponentGoneEvent;",
        ),
        raw(
            "AppData",
            "export interface AppData<S = Settings> {\n  settings: S\n  accounts: LichessAccount[]\n  gameCount: number\n}",
        ),
        raw("CoreData", "export type CoreData = AppData<CoreSettings>"),
    ]);
    out
}
