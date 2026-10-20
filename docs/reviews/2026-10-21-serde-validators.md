# Typed serde validators in place of the valibot port (2026-10-21)

`crates/kchess-domain/src/misc/engine.rs` re-implemented valibot 1.5 over a JS-like value (`J`)
so the Rust validators reported exactly what the deleted TypeScript did. Phase 1 moved the IPC
validators (`misc/validate.rs`) to typed `#[derive(Deserialize)]` structs with explicit checks.
Phase 2 moved the six document checks the same way and deleted `engine.rs`.

## Phase 1: the IPC validators

- `misc/value.rs` holds `J` and a serde `Deserializer` over it. Its errors carry the path of the
  rejected value. Each validator maps a path to its custom message; without one, the decoder's
  own text is used, prefixed by the path (`speeds[0]: …`).
- `undefined` is an absent optional field. `null`, NaN and binary data match no field type.
  Integral numbers decode as integers. Lists and `Uint8Array`s still decode as objects of their
  indices, as `typeof value === 'object'` did in valibot. Integral outputs are still written as
  integers.
- Every custom message is kept. `misc/patterns.rs` and `js.rs` stay: the new checks and the
  document decoders still use them.

### Behaviour changes

Two replays compared old and new outcomes: one ran the seeded cases of
`tests/core/native-misc.test.ts` against the old (HEAD) and new Rust, and one ran an 11,000-case
mutation corpus over every validator. Neither found an accept/reject flip or a different
normalized value. Only messages changed:

1. **Default messages.** Where the TypeScript gave no custom message, valibot's text is replaced:

   | valibot (before)                                        | now                                                         |
   | ------------------------------------------------------- | ----------------------------------------------------------- |
   | `Invalid key: Expected "offset" but received undefined` | `` missing field `offset` ``                                |
   | `Invalid key: Expected never but received "x"`          | ``unknown field `x`, expected one of …``                    |
   | `Invalid type: Expected string but received 1`          | ``name: invalid type: integer `1`, expected a string``      |
   | `Invalid type: Expected ("win" \| …) but received "x"`  | ``result: unknown variant `x`, expected one of …``          |
   | `Invalid length: Expected <=60 but received 61`         | `password: invalid value, expected at most 60 characters`   |
   | `Invalid value: Expected >=1 but received 0`            | `invalid value, expected a number from 1 to 5`              |
   | `Invalid format: Expected /^…$/ but received "x"`       | `since: invalid value, expected a year or a year and month` |
   | `Invalid finite: Received Infinity`                     | `invalid value, expected a finite number`                   |
   | `Invalid UUID: Received "x"`                            | `invalid value, expected a UUID`                            |
   | `Invalid type: Expected (Uint8Array \| string) …`       | `data: invalid type, expected a string or a Uint8Array`     |

   Validators with default messages: `assertNewArena`, `assertExport`, `assertGamePageQuery`,
   `assertRendererError`, `assertBroadcastQuery`, `assertLookupOptions`, `assertLines`,
   `assertLookupKind`, `assertOptionalPage`, `assertBoolean`, `assertChapterName`,
   `assertPgnText`, `assertSessionObject`, `assertUuid`, `assertPerformanceName`,
   `assertTimingValue`, and some fields of `assertInsightsQuery` (`speed`, `rated`),
   `assertAnalysisRequest` (`rootFen`, FEN length), `assertVoiceAttempt` (lengths, scores,
   `words`), `assertVoiceUpdate` (`expected`), `assertRunInput` (`score` range, `detail` values)
   and `assertTheme` (a palette's shape).

2. **Which fault is reported first.** valibot reported the first issue in schema order. Now type
   errors and missing fields come first (type errors in the order the input lists its
   properties, then missing fields), then the checks in field order. The message is still one
   that applies to the input. Example: `{ kind: 'e2e4' }` for `assertRunInput` reported
   `Invalid kind of run.` and now reports `Invalid run.` (missing `variant`).

### Tests and goldens updated

- `tests/unit/validate-rules.test.ts`: three assertions on valibot defaults (`'Invalid key'` ×2,
  `'Invalid type'`).
- `tests/core/golden/misc.json`: re-recorded only the 13 suites with changed outcomes. 225 of
  6,255 cases changed: 222 default messages and 3 ordering cases (`assertOnlineOptions` #12,
  `assertPuzzleSolve` #50 and `assertRunInput` #45). By suite: `assertGamePageQuery` 51,
  `assertExport` 49, `assertNewArena` 47, `assertBroadcastQuery` 34, `assertLookupOptions` 15,
  `assertInsightsQuery` 12, `assertAnalysisRequest` 6, `assertVoiceAttempt` 4, `assertTheme` 3,
  `assertVoiceUpdate` 1, plus the three ordering cases. No other suite changed.
- `tests/core/golden/store-games.json`: one case, `seed 90210` #90 (a `gamePage` call with
  `result: 'other'`), from valibot's default message to
  `` result: unknown variant `other`, expected one of `win`, `loss`, `draw` ``. No other golden
  file changed.
  `tests/core/native-golden.json` is untouched.

## Phase 2: the document checks (approved 2026-10-21)

### What used the engine after Phase 1

Only six checks in `misc/documents.rs`, all with valibot default messages. They now live in
`validate.rs` with the other `assert*` validators:

| check                     | now                                                       | pinned cases (ok / error) |
| ------------------------- | --------------------------------------------------------- | ------------------------- |
| `assertStudyCommandShape` | the `op` decodes as an enum, then one struct per command  | 34 / 68                   |
| `assertLegacyDocuments`   | a `deny_unknown_fields` struct of the legacy storage keys | 53 / 38                   |
| `assertLibraryId`         | a string of 1–80 characters                               | 61 / 30                   |
| `assertRepertoireKey`     | a string of 1–200 characters                              | 66 / 25                   |
| `assertSessionKind`       | one of the session kinds                                  | 3 / 88                    |
| `assertSide`              | `white` or `black`                                        | 3 / 88                    |

A study to restore is passed on as it is, as valibot's `unknown()` did. `js_string`
(`String(color)` in `decodeComputerSession`) moved into `documents.rs`. `own_entries` moved
into `value.rs`.

### What was left alone

- The session decoders (`decodeAnalysisSession`, `decodeLocalSession`,
  `decodeComputerSession`, `decodeArchiveIdentity`, `decodeJoinedTournaments`,
  `decodeRepertoireMisses`, `encodeSession`, `assertSession`) are hand-written walks over `J`
  that salvage damaged documents field by field. They never used the engine. Rewriting them as
  all-or-nothing typed structs would reject partly damaged documents they now recover.
- `native-golden.json` pins `library.rs`, which decodes over `serde_json::Value` with `js.rs` and
  never used the engine either.
- `js.rs` and `patterns.rs` stay: the validators and decoders still use them.

### Behaviour changes

The old/new replays show no accept/reject flip and no different normalized value: neither the
seeded `native-misc` cases nor the mutation corpus, which was extended with cases for the six
checks. Only default messages changed, for example:

| valibot (before)                                                 | now                                                           |
| ---------------------------------------------------------------- | ------------------------------------------------------------- |
| `Invalid type: Expected ("save" \| …) but received "x"`          | ``op: unknown variant `x`, expected one of `save`, …``        |
| `Invalid type: Expected ("save" \| …) but received undefined`    | `op: invalid type: undefined, expected a study command`       |
| `Invalid key: Expected "name" but received undefined`            | `` missing field `name` ``                                    |
| `Invalid key: Expected never but received "x"`                   | ``unknown field `x`, expected one of `kchess:studies:v1`, …`` |
| `Invalid length: Expected <=80 but received 81`                  | `invalid value, expected at most 80 characters`               |
| `Invalid type: Expected ("white" \| "black") but received "red"` | `invalid value, expected one of white, black`                 |

Type errors are reported before length and pattern checks, as in Phase 1. None of these
checks had custom messages.

Phase 2 also fixed the grammar of two default texts shared with Phase 1: "at least 1
character" and "at least 1 item" (they said "characters" and "items"). The only validators
whose default texts this changes are `assertChapterName` and `assertPgnText` (an empty string)
and an empty chapter list in `assertStudyCommandShape`. No golden pins them.

### Goldens

- `tests/core/golden/misc.json`: 337 cases re-recorded, exactly the six suites' pinned errors.
  By suite: `assertSessionKind` 88, `assertSide` 88, `assertStudyCommandShape` 68,
  `assertLegacyDocuments` 38, `assertLibraryId` 30, `assertRepertoireKey` 25. Every accepted
  case and every other suite is unchanged.
- `tests/core/native-golden.json`: unchanged.
