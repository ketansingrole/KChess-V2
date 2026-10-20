# Typed serde validators in place of the valibot port (2026-10-21)

`crates/kchess-domain/src/misc/engine.rs` re-implemented valibot 1.5 over a JS-like value (`J`)
so the Rust validators reported exactly what the deleted TypeScript did. This change moves the IPC
validators (`misc/validate.rs`) to typed `#[derive(Deserialize)]` structs with explicit checks.
The document checks in `misc/documents.rs` still use the engine; Phase 2 below proposes how to
remove it and needs a decision.

## Phase 1: the IPC validators (this change)

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

## Phase 2 proposal: the document checks (needs a decision)

### What still uses the engine

After Phase 1, `engine.rs` serves only six checks in `misc/documents.rs`, all with valibot
default messages:

| check                     | schema                                    | pinned cases (ok / error) |
| ------------------------- | ----------------------------------------- | ------------------------- |
| `assertStudyCommandShape` | `variant('op', …)` over 12 study commands | 34 / 68                   |
| `assertLegacyDocuments`   | strict object of the legacy storage keys  | 53 / 38                   |
| `assertLibraryId`         | string, 1–80                              | 61 / 30                   |
| `assertRepertoireKey`     | string, 1–200                             | 66 / 25                   |
| `assertSessionKind`       | picklist of session kinds                 | 3 / 88                    |
| `assertSide`              | `white` \| `black`                        | 3 / 88                    |

It also uses two helpers: `js_string` (`String(color)` in `decodeComputerSession`) and
`own_entries` (now in `value.rs`).

### What does not use it

The brief assumed the engine also decodes saved documents. It does not:

- `native-golden.json` (`studies`, `mistakes`, `archive`, `archivedGame`, `study`, …) pins
  `library.rs`, which decodes over `serde_json::Value` with `js.rs`. Removing the engine changes
  **none** of those cases.
- The session decoders (`decodeAnalysisSession`, `decodeLocalSession`,
  `decodeComputerSession`, `decodeArchiveIdentity`, `decodeJoinedTournaments`,
  `decodeRepertoireMisses`, `encodeSession`, `assertSession`) are hand-written walks over `J`
  that salvage damaged documents field by field. They have no schema and no messages.
- `training/data.rs` and `contracts.rs` do not use the engine (`contracts.rs` only uses `J`).

### Proposed change

Port the six checks to typed serde, the same way as Phase 1: a `#[serde(tag = "op")]` enum for
study commands, a `deny_unknown_fields` struct for legacy documents, and plain checks for the
four scalars. Then delete `engine.rs`. Keep the session decoders as they are: rewriting them as
all-or-nothing typed structs would reject partly damaged documents they now recover, which is a
real behaviour change for users' saved libraries and not needed to remove the engine.

### Golden impact if approved

- `tests/core/golden/misc.json`: up to **337 cases** in the six suites above (every pinned
  error), all valibot default messages, would get the new default texts. The 220 accepted cases
  would stay identical if acceptance parity holds (verify with the same old/new replay).
- `tests/core/native-golden.json`: **0 cases**.
- Ordering can also differ for `assertStudyCommandShape`. valibot's `variant` picked the first
  option whose `op` matched and reported that option's first issue in schema order. Serde would
  report a type error first.

Rewriting these goldens needs explicit approval (AGENTS.md: golden rewrites only for intended
behaviour changes).
