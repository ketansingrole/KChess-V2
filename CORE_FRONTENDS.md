# Core and frontend ownership

The headless core is a Node library. It owns credentials, network policy, engines, puzzles,
settings and versioned documents. A frontend owns input and presentation. It calls `CoreApi`
and subscribes to `CoreEvents`; it does not open the database or spawn an engine itself.

| Layer                                                          | Responsibility                                                                  |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `crates/kchess-wasm/js`                                        | Pure chess/training rules, validation, game controllers and archive transitions |
| `crates/kchess-node/js`                                        | Privileged services, document persistence, networking and engine scheduling     |
| `apps/desktop/electron/main` + `apps/desktop/electron/preload` | Electron host capabilities and authenticated desktop IPC                        |
| `apps/desktop/app`                                             | Vue views and reactive mirrors, board display, sounds and notifications         |
| `hosts/node/src`                                               | Independent worker hosts, OS credentials and browser login                      |
| `apps/cli/src`                                                 | Terminal commands, input, output and persistence through the core               |

## Node library

Build with `pnpm run build:native` and `pnpm run build:packages`, then import the generated entry:

```js
import { createNodeCore } from '@kchess/node'

const core = await createNodeCore({ dataDir: '/absolute/path/to/profile' })
const off = core.on('engine:analysis', (update) => console.log(update))
try {
  // Required before engine assistance: recover/verify live Lichess game state.
  await core.resumeOnline()
  const move = await core.bestMove(['e2e4'], 'beginner')
  console.log(move)
} finally {
  off()
  await core.close()
}
```

Create multiple clients with different profile directories. Each gets a separate worker,
module state, SQLite connection, puzzle service and engine scheduler. Their methods are
asynchronous. The Node host also exposes asynchronous `settings`, `trustEnginePath`,
`voiceHistoryDocument` and `suspend` helpers. Diagnostic output goes to stderr so CLI JSON
on stdout remains usable in pipelines. A failed worker rejects outstanding calls.

For embedded hosts, `crates/kchess-node/dist/index.js` exports `createKChessCore(CorePlatform)`.
Its platform supplies existing data directories, engine paths, a SecretStore, browser
opening and battery state; focus and legacy paths are optional. Direct cores may coexist in one runtime when their profile directories differ.
Each owns its database, caches, queues, engine scheduler and workers. A second core for
the same profile is rejected until the first has finished `close()`. Calls retain the original platform through
an async scope so a stale callback cannot access another profile.

`VoiceModelCache` (from the same entry) downloads, verifies and caches the offline Vosk model
in a directory the host chooses. Speech recognition and microphone capture stay in the frontend:
desktop runs `vosk-browser` in the renderer and maps spoken text with `crates/kchess-wasm/js/voiceCommands`.

## Game controllers

`crates/kchess-wasm/js/dist/gameSession.js` exports `ComputerGame`, `LocalGame`, and their state factories.
They accept plain mutable state; desktop wraps that state with Vue reactivity. The computer
controller receives engine methods, readiness/assistance checks, a monotonic clock, and
callbacks for move sounds, notifications and errors. Its cancellation stops the actual
engine search and rejects late replies after reset, takeback, disposal or revoked assistance.

Both controllers preserve root setup and complete move history for repetition and variants.
They settle clock expiry before a move/increment, and expose session and archive snapshots.
`GameArchive` shares stable identities, takeback removal and completion behavior between
Vue and terminal games. Frontends persist snapshots through the core library. Desktop quit requests first flush
renderer persistence owners and acknowledge the shell; core shutdown then runs before
Electron closes the windows. This preserves debounced edits and keeps Node cleanup out
of the final `will-quit` hook.

## Shared online and puzzle sessions

`crates/kchess-wasm/js/dist/onlineGame.js` exports `OnlineGame` and `onlineGameState`. It reduces authoritative
Lichess events into plain state, owns clock interpolation, offers, opponent identity and
rematch history, and never retries an ambiguous move. Hosts provide the CoreApi transport,
a monotonic clock and optional presentation callbacks. Vue wraps the same state reactively.

`crates/kchess-wasm/js/dist/puzzleSession.js` exports `PuzzleSession` and `puzzleSessionState`. It owns offline
difficulty selection, online/practice fallback, attempt identity, exactly-once result
submission, session totals and stale-response rejection. Account/mode/theme choices come
from the host; retry attempts never submit a rated result. Board display and navigation
remain frontend responsibilities.

## Settings contracts

`CoreSettings` contains only service preferences. CoreApi reads, mutation results and
settings events expose that shape; CLI writes need no colors, sounds or update preferences.
`FrontendPreferences` defines presentation and desktop settings. The Electron adapter
combines both as `Settings` for its existing UI. Saved desktop preferences retain their
legacy database columns and survive headless settings updates.

## CLI

Run `pnpm run cli -- --help` for commands. `call` accepts any CoreApi method and a JSON
argument array; `--args-file` avoids shell quoting for large PGNs/documents. Argument tuples
and domain validation run in the core, independent of transport. Core events drive
`analyze` output; interactive games accept UCI moves, undo, resignation and new games.
Their unfinished sessions and archive identities survive subsequent CLI launches.

Use `--engine /absolute/path/to/stockfish` to explicitly trust and save a native engine.
Without it, the bundled Stockfish WASM runs under Node. `--browser print` supports remote
terminal login. Browser URLs still use OAuth state and a bounded local callback server.
The default Node profile is separate from Electron because its OS credential encryption
format differs. macOS uses a per-profile AES key in Keychain, Linux stores that key in
Secret Service, and Windows encrypts with DPAPI for the current OS user. There is no
plaintext credential fallback; `--no-credentials` disables login storage.

## Verification

`pnpm run check` covers architectural import rules, Node-only types, shared and desktop
unit regressions, direct-core smoke tests, two concurrent Node profiles, subscriptions,
real engine searches, the puzzle service, CLI input/output, persistence and validation.
Production Electron and packaged checks remain required for desktop changes. OAuth and
OS-keychain integration require a real account/user session; automated fixtures never
open a browser or access the user's credentials.
