# Shared core: one core per profile, any number of KChess windows

Status: built, 2026-10-26. Its clients are the app and the CLI (`kchess-cli`). Stages 1–6 have
shipped, except that the app still accepts `KCHESS_CORE_MODE=embedded` (the end-to-end tests use
it) and there is no MCP client. Open findings are in the
[26 October review](reviews/2026-10-26-code-review.md).

## Why

Before this, every KChess app embedded its own core on a Tokio runtime inside the app
(`apps/gpui/src/core.rs`).
The core is the profile's only writer, and `ProfileClaim` (`crates/kchess-core/src/facade.rs`)
enforces that with an advisory `profile.lock`: a second app on the same profile fails with
"A KChess core is already running for this profile."

That rule protects the data, but it limits what people can do:

- Two KChess windows cannot share a profile.
- Our own future tools (a CLI, an MCP server) cannot reach the profile while an app is open.

The goal is to keep **exactly one core per profile** and let **any number of clients** use it.

### Decisions already made

- **A shared core service, started on demand.** Not one core per app writing a shared database
  (the core also holds the Lichess connection, the engine scheduler, credentials and caches), and
  not an always-on OS service (installation, updates and uninstall on three platforms, and it would
  run when nobody uses KChess).
- **The socket is internal.** Only clients shipped in the same KChess release connect to it. There
  is no public extension API and no compatibility promise. Every method stays in
  `CoreApi`/`CORE_METHODS` and the types stay generated from `kchess-contracts`, so a public subset
  could be cut later without redesign.

## Overview

```
 GPUI app ──────┐   local socket (per profile, per user)   ┌──────────────────────────────┐
 CLI ───────────┼─────────────────────────────────────────▶│ kchess-service serve         │
 MCP (later) ───┘   framed JSON: calls, results, events,   │   ProfileClaim, profile.lock │
                    cancellation                           │   Core (kchess-core)         │
                                                           └──────────────────────────────┘
```

- `kchess-service serve --profile <dir>` is a small binary around `Core`. It holds the profile claim
  and lock exactly as an embedded core does.
- Clients never open the profile's files. They speak to the service.
- Pure rules stay in-process: `kchess-domain` is stateless, so only stateful core methods go over
  the socket.

## The service

### What it is

- A Rust binary in its own crate (`crates/kchess-service`, `src/bin/kchess-service.rs`) that
  builds a `Config`, calls `Core::open`, and serves connections. The `Host` it passes to the core
  (`crates/kchess-core/src/host.rs`) routes `emit` to connections and `log` to the service's own
  log file in the profile.
- Shipped inside every app package, beside the app's executable, the bundled Stockfish and
  `kchess-voice`. It never runs from there: see [Install, update and uninstall](#install-update-and-uninstall).
- Signed with the app's identity, so the keychain sees the service as part of KChess. Development
  builds are signed by `tooling/macos-dev-runner.sh` the same way as the GPUI app, so
  "Always Allow" survives rebuilds.

### What moves into it

- Everything `Core` owns today: the profile database, library, `puzzles.db`, settings, accounts and
  credentials, Lichess streams, engines (`EngineContext::lease`), reviews and voice recognition.
- Host capabilities the service can answer itself: `onBattery` (the GPUI app already reads it
  natively in `power.rs`).

### What stays in the apps

- Windows, menus, the Dock, desktop notifications, the browser, the microphone permission prompt.
- Host capabilities that need a window: `openExternal` and `focus`. See
  [Routing host requests](#routing-host-requests).

## The socket

### Where it lives

- macOS and Linux: a Unix domain socket. `sun_path` is limited to about 104 bytes, and profile
  paths (`KCHESS_USER_DATA_DIR`, long home directories) can exceed it, so the socket lives in a
  per-user runtime directory (`$XDG_RUNTIME_DIR`, else `$TMPDIR`), named after a hash of the
  profile's canonical path: `kchess-<hash>.sock`.
- Windows: a named pipe, `\\.\pipe\kchess-core-<hash>` (a hash of the user name and the profile).
- The service writes `core.json` in the profile: socket path, process id, release, protocol hash
  and start time. Clients read it to find the service; the lock, not the file, decides whether a
  service is alive.

### Who may connect

- The socket's directory is `0700` and the socket `0600`; every instance of the named pipe is
  owned by the current user with a protected DACL granting that user alone (`transport.rs`).
- On Unix the service checks the peer's user id (`getpeereid` / `SO_PEERCRED`) and refuses
  anyone else; on Windows the pipe's DACL keeps other accounts from opening it, and remote
  clients are rejected.
- Pipe names are one namespace for the whole machine, so another account could create the name
  first. A client refuses a pipe its own user does not own before it sends anything, and the
  service deletes a dead core's `core.json` and `core.token` before it binds.
- The `hello` frame is read with a 64 KiB limit and at most 32 connections may be in their
  handshake at once, so a client that has not proven itself cannot make the service allocate much.
- A `host:request` sent to one client is answered by that client alone.
- The first frame must carry the token from `core.token` in the profile (32 random bytes, `0600`,
  replaced on every service start). Any process that can read the profile could already read the
  database, so the token adds no new trust; it stops other users' processes and confused local
  software from talking to the core.

### Frames

- Length-prefixed UTF-8 JSON (4-byte big-endian length, then the body), capped at 16 MiB per
  frame. Library documents are already bounded (`store/library.rs`), so no reply comes close.
- Messages:
  - `hello` (client to service): token, client kind (`gpui`, `cli`, `mcp`), release,
    protocol hash, capabilities it can answer (`openExternal`, `focus`, `secrets`).
  - `welcome` / `refuse` (service): the connection id, or why not (see
    [Version check](#version-check)).
  - `call`: `{ id, method, args }`, exactly what `Core::call` takes today.
  - `result`: `{ id, value }` or `{ id, error }`, with the core's `CoreError` message unchanged.
  - `cancel`: `{ id }`. Cancellation must reach the work (the repository rule for `UciController`
    and `RequestScope`), so the service maps it to the task's cancellation, not to dropping the
    reply.
  - `event`: `{ name, payload }`, the `CoreEvents` names unchanged, plus the new change events
    below.
  - `host:request` / `host:reply`: the core's capability round trip (`HostAsk` and `HostReply`
    in `kchess-contracts/src/host.rs`), addressed to one connection.
  - `focus`: `{ focused }`, the client's window gained or lost focus.
  - `suspend`: the client has no window open; what it started stops, and every other client's
    work carries on. No client closes the shared core: it closes once idle.
- Backpressure: each connection has a bounded outgoing queue. High-rate events (`engine:analysis`,
  `watch:frame`, `puzzledb:progress`) are coalesced per connection (the latest per session wins)
  when a client falls behind; replies and every other event are never dropped. A client that stays
  full past a deadline is disconnected and reconnects.

## Lifecycle

### Start

An app always reuses what is already there. It copies or starts something only when it must:

1. **A core is running for the profile:** the app reads `core.json` and connects. Nothing is
   copied or started. This is the normal case for the second app, a second window and every tool.
2. **No core is running, and this release's copy exists** in the runtime folder (see
   [Where the service runs from](#where-the-service-runs-from)): the app checks the copy's hash
   and starts it. Nothing is copied.
3. **No core is running and there is no copy of this release:** the app copies its bundled binary
   (first launch after installing or updating), then starts it.

Starting means `kchess-service serve --profile <dir>`, detached from the app (its own process group or
session, so closing the window that started it does not end it); the app then connects, retrying
with backoff for up to 5 seconds.

- Two apps may find no core at the same moment and both start one. The profile lock decides: the
  second service fails its claim and exits, and both apps connect to the first.
- The service writes `core.json` and opens the socket only after the claim succeeds and the core is
  open, so an app never reaches a half-started core.
- A running core from another release is never replaced while other apps use it, and an older or
  newer copy is never started in place of the app's own release: see
  [Version check](#version-check).

### Running

- The service stays up while at least one client is connected.
- Work today's apps do in the background (syncs, reviews, an online game in progress, a puzzle
  database download) keeps running for as long as any client is connected, as now.

### Stop

- When the last client disconnects, the service waits 10 seconds (a window reloading, an app
  restarting after an update, a CLI command run twice), then runs the core's existing `close` and
  releases the profile.
- Before a window closes it still flushes its unsaved work through the core (`core::on_quit`).
  Closing the core is no longer the window's job.
- An online game when the last window closes: the same as quitting the app today, because the
  core closes after the grace period. Keeping a live game alive with no window to show it would
  be a separate decision.

### Crashes

- If the service dies, every client sees its socket close. Calls in flight fail with a "core
  restarted" error, and clients start a new service and reconnect, showing "Reconnecting…" in the
  meantime.
- A failed call is retried automatically only if it reads. Mutations are not, and an ambiguous
  online move mutation is never retried (the repository rule for Lichess).
- After reconnecting, a client refetches what it shows: settings, library, accounts, the online
  state.

## Multi-window sync

The core today assumes one caller. Three kinds of method need different treatment.

### Profile data: shared, with change events

Library documents, settings, accounts, puzzle progress and view preferences belong to the
profile. Every window sees the same data, so a write in one window must reach the others.

- After each successful write the core broadcasts `profile:changed { areas, origin }`: the parts
  of the profile it changed (`ProfileArea` in `kchess-contracts`: studies, played games, drills,
  sessions, tournaments, repertoire, Lichess games, accounts, desktop settings, puzzles, voice
  history, usage, engine, view preferences) and the connection that wrote. `settings:saved` already reported
  settings, and still does.
- A window ignores its own writes (`origin`) and refetches what it shows: its studies, played
  games and accounts (`core::on_profile_change`). It adopts another window's view preferences at once, except names they changed themselves since reading. Events name areas, not
  documents, so the core stays the source of truth. A window with its own edits still to save
  keeps them and shows the library once they are saved.
- Concurrent edits: `studyCommand` takes an optional `basedOn`, the chapter's PGN as the window's
  board last read or wrote it. The library (`apps/gpui/src/studies.rs`) keeps it per chapter and send it with every chapter save; a library refresh does not move it, so a board
  still showing older moves stays protected. A save whose chapter has changed since is refused
  ("This study was changed in another KChess window. Reload it to see the latest version.")
  instead of overwriting the other window's, and is not retried: reopening the study adopts the
  newer moves. A save whose study or chapter was removed since is refused the same way, rather
  than recreating it. A save with a `basedOn` writes only the chapter's moves: the study keeps
  its stored name. Renames and other chapters do not conflict. The documents' `version` field is the
  schema version and stays as it is.
- View preferences (tabs, sort order, layout) are per-profile and last-write-wins. They are read
  when a page opens and are deliberately not broadcast, so one window's tab change never moves
  another window's tabs.

### Sessions: per connection

Some methods hold one caller's activity, and today replace the previous one:
`startAnalysis`/`stopAnalysis` ("replacing any running analysis"), `watch` and `watchBroadcast`
("replacing what was watched"), and `voiceStart`/`voiceStop`.

- **Watching** is per connection: two windows watch two channels or broadcasts at once, and
  `watch:*` events go only to the window that asked.
- **Analysis** is per connection, and its `engine:analysis` updates go only to the window that
  started it. The engine scheduler gives its whole thread budget to one lease at a time
  (`EngineContext::lease`), so the newest request wins: starting an analysis interrupts any other
  window's (a final `interrupted` update), and that window restarts it when the person returns to
  it. A window's `stopAnalysis` stops only its own.
- **Voice input** has one owner: the window that called `voiceStart` last. Audio from another
  window is refused ("Voice input is in use in another KChess window.") until the owner stops or
  disconnects.
- A connection closing ends its watch and analysis and releases voice input.
- Analysis updates already carry the client's `clientId`; with one analysis per connection, the
  ids never meet another window's, so caches keep distinguishing clients as the repository rules
  require.

### Live play: one, shared

An account has at most one online game, one seek and one set of challenges, and the core owns the
Lichess stream. These are shared:

- `online:state`, `online:event`, `online:lobby`, `challenges:update` and
  `online:ongoing-changed` go to every connection, so every window shows the same game.
- Moves and actions are serialised in the core. Two windows cannot both send a move for the same
  position; the second is refused, never queued.
- Fair play holds across connections: while a live human game is in progress, no connection gets
  engine analysis, review or explorer data, whichever window or tool asks. The core already owns
  this rule and must check it for every connection, not only the window that is playing.

### Alerts

`challenge:received` and similar events tell frontends to alert. With several windows, each would
show its own system notification.

- The core names one connection the alert owner and broadcasts it (`alerts:owner`): the window
  focused last (clients report focus changes), else the window that connected last. Clients
  without windows never own alerts. A window raises system notifications only while it is the
  owner (or no owner is known); the others update silently.

### Routing host requests

`host:request` (`crates/kchess-core/src/capabilities.rs`) asks the host for something only a
window can do.

- A request caused by a call goes to the connection that made the call (signing in to Lichess
  opens the browser from the window that asked).
- `focus` goes to the alert owner.
- A CLI or MCP client declares no window capabilities and never receives these requests.

## Version check

KChess apps from different releases do not share a core. A person running two apps updates the
older one; nothing tries to make two versions work together.

- Every build computes a **protocol hash**: a hash of the method names and parameters
  (`METHODS`), the event names (`CoreEvent::NAMES`) and the error kinds in `kchess-contracts`,
  plus the framing version. Clients built from the same release always agree.
- `hello` carries the client's app, release and protocol hash.
  - Same hash: `welcome`.
  - Different hash, and the client is the only connection: the service closes and the client
    starts its own release's service. Database migrations run as today when a newer core opens the
    profile.
  - Different hash while other clients are connected: `refuse`, with the connected apps and their
    releases. The window that cannot connect tells the person which app to update:
    - this app is older: "KChess 2026.11.0 (GPUI) is open on this profile. Update this app to
      2026.11.0 to use them together.", with the app's own Update button (Settings → Updates);
    - this app is newer: "KChess 2026.11.0 is open on this profile. Update it to 2026.11.1, or
      quit it, to open this app."
- A profile migrated by a newer release cannot be opened by an older core, even alone (as today).
  The older app says "This profile was used by KChess 2026.11.0. Update to 2026.11.0 to open it."

## Clients

- Both frontends hold their core through `kchess_service::CoreHandle` (`handle.rs`): one shared
  or embedded backend, one event channel, typed calls, host replies, focus, `suspend` and `close`
  with one meaning each, and, for the app, a reconnect after the service stops. Both configure it
  through `kchess_service::Layout` (`layout.rs`): the profile, the engines, the speech helper,
  the service binary and the test switches, read once as the frontend starts.
- **GPUI.** `CoreBridge` (`apps/gpui/src/core.rs`) runs the handle on its runtime; views do not
  change. `answer_host` answers routed `host:request`s on the UI thread.
- **CLI** (`apps/cli`), **and MCP (later).** Socket clients with no window capabilities, whose host
  requests the handle answers as they arrive. If no service runs, they start one; it stops 10
  seconds after they disconnect.
- **Embedded mode for tests.** `KCHESS_CORE_MODE=embedded` keeps an in-process core for tests that
  need no second process (the app's end-to-end tests).

## Install, update and uninstall

Nothing changes for the person installing KChess: no extra installer, no administrator rights, no
system service, no background process when no app is open. The core is part of each app's
download, as it is today.

### Size

- The core lives in the service binary; the app keeps only what it calls directly (rules, the
  socket client).
- Two installed copies of KChess hold two copies of the service binary on disk. Only one ever runs
  per profile.

### Where the service runs from

The service never runs from inside an app's folder. A core still serving one client must not keep
an app's folder in use, because:

- Windows cannot replace or rename a running executable, so the installer the updater runs
  (`apps/gpui/src/updates.rs`) would fail while the service is open;
- an AppImage's files live on a mount that disappears when the app that started the core quits;
- deleting an app (dragging it to the Bin, uninstalling) would pull the core out from under the
  other app.

So before starting the service, a client copies its bundled binary to a per-version folder in the
user's application data, outside any profile, and starts it from there:

| Platform | Folder                                                         |
| -------- | -------------------------------------------------------------- |
| macOS    | `~/Library/Application Support/KChess Runtime/core/<release>/` |
| Windows  | `%LOCALAPPDATA%\KChess Runtime\core\<release>\`                |
| Linux    | `~/.local/share/kchess-runtime/core/<release>/`                |

The default profile on macOS is `~/Library/Application Support/KChess`, so the runtime folder is a
sibling of it, never inside it: a profile can be copied, backed up or pointed at with
`KCHESS_USER_DATA_DIR` without carrying executables.

- The copy is written to a temporary name, checked against the hash the app was built with, and
  renamed into place, so two apps copying at once cannot leave a broken file. An app whose release
  is already there checks the hash and uses it.
- The copy keeps its code signature, so on macOS the keychain sees the same KChess identity before
  and after an update, and "Always Allow" lasts across releases. Files an app writes are not
  quarantined, so Gatekeeper does not question it.
- On start, the service removes other releases' folders that no running service uses. At most two
  releases stay on disk, briefly, during an update.

### Updates

- An app updates as today, with its own updater or installer. Its folder is free because the core
  runs from the copy.
- After the update, the app's next start finds the older core still running for another app and
  shows the version message above. The person updates the other app too; until then, one app at a
  time.
- A core running alone for an app that updated restarts into the new release when that app
  reconnects (the sole-client rule above).

### Uninstall

- Uninstalling an app leaves the profile and the copied core as today's uninstall leaves the
  profile. The other app keeps working with its own release's copy.
- Uninstalling the last app leaves the runtime folder behind with the profile; removing KChess's
  data removes both.

## Testing

- Two clients on one profile: a change in one reaches the other (`library:changed`), sessions stay
  separate, and live play is shared.
- Two clients starting at the same moment end with one service.
- The service killed mid-call: reads retry, mutations report the failure, clients reconnect.
- Version mismatch: a sole client replaces the service, and with others connected it is refused,
  naming the other app and the release to update to.
- Install and update: the copy is made once per release and survives a concurrent copy; the updater
  succeeds while another client keeps using the core; an
  AppImage that started the core can quit without stopping it; old releases' copies are removed.
- Security: another user's process, a missing token and a stale token are refused.
- Fair play: with a live game in one window, analysis from another connection is refused.
- Performance budgets: a socket round trip under 1 ms at the 95th
  percentile, `engine:analysis` delivered at the engine's rate with coalescing only under load,
  and startup to the first window no slower than with the embedded core.
- Packaged tests (`cargo xtask package`, and CI's packaging jobs): the service binary is present,
  signed and starts from every installer.

## Staged plan

All shipped: connection-scoped sessions in `Core`; the service, socket, version check and
per-release copy; the app as a client; windows kept in sync by the change events; and the CLI as
a plain socket client. What remains is an MCP client, and a public extension API (a chosen subset
of `CORE_METHODS` with its own version) if one is ever wanted. The stage-by-stage plan, including
the Electron app's move onto the service, is in Git history.

## Open questions

- Should a live online game keep going when the last window closes? Today quitting ends it; the
  service could keep it for the grace period only, or until the game ends.
- Do background reviews and syncs ever run with no window open? This plan says no.
- Linux without `$XDG_RUNTIME_DIR` (some containers): `$TMPDIR` with a per-user subdirectory, or the
  profile itself when its path is short enough.
