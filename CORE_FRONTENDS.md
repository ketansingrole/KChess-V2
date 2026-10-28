# Core and frontend ownership

The headless core (`crates/kchess-core`) owns credentials, network policy, engines, puzzles,
settings and versioned documents. A frontend owns input and presentation. It calls `CoreApi`
(`crates/kchess-contracts/src/core_api.rs`) by method name and subscribes to the core's events; it
does not open the database or spawn an engine itself.

| Layer                    | Responsibility                                                          |
| ------------------------ | ----------------------------------------------------------------------- |
| `crates/kchess-domain`   | Pure chess, training and document rules, validation of every argument  |
| `crates/kchess-games`    | Computer, over-the-board and Game history sessions over those rules    |
| `crates/kchess-core`     | Privileged services, document persistence, networking, engine scheduling |
| `crates/kchess-service`  | The profile's shared core over a local socket                           |
| `apps/gpui`              | Views, the board, sounds, notifications, microphone capture             |
| `apps/cli`               | Terminal commands, input and output                                     |

## Reaching the core

`kchess_service::connect_or_start(profile, hello, start)` connects to the profile's core service,
starting it from the binary shipped beside the frontend when none runs (`docs/shared-core.md`).
`Client::call(method, args)` answers with JSON; events arrive on the receiver it returns.
`host:request` events ask the frontend for a capability it declared in `hello`; it answers with
`host.reply(id, { value } | { error })`.

A frontend can instead open a core in its own process (`kchess_core::Core::open(config, host)`),
as `kchess-cli --embedded` does; only one core may hold a profile at a time (`profile.lock`).

## Game sessions

`kchess_games::computer::ComputerGame`, `local::LocalGame` and `archive::GameArchive` run the
domain's transitions: the frontend answers their reads (the clock, whether engine help is
allowed) and carries out their effects (searches, sounds, saving the session and its Game history
entry). The app and the CLI share them, so a game started in one carries on in the other.
