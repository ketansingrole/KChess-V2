# Core and frontend ownership

The headless core (`crates/kchess-core`) owns credentials, network policy, engines, puzzles,
settings and versioned documents. A frontend (the app, the CLI) owns input and presentation; it
does not open the database or spawn an engine itself. The crates are mapped in `ARCHITECTURE.md`.

## Reaching the core

`kchess_service::CoreHandle::open(Options { config, hello, mode, reconnect, log, answer })`
(`crates/kchess-service/src/handle.rs`) opens the profile's core and returns the receiver its
events arrive on. With `Mode::Service` it joins the profile's core service, starting it from the
binary shipped beside the frontend when none runs (`docs/shared-core.md`); with `Mode::Embedded`
the core runs in the frontend's own process, as `kchess-cli --embedded` does. Only one core may
hold a profile at a time (`profile.lock`).

A frontend calls the core with typed calls: `CoreHandle::request(call)` takes one of
`kchess_contracts::calls` and answers in its shape. A call by method name is plumbing (the CLI's
`call` command takes any method); xtask's `boundaries` test refuses it elsewhere.

Host requests ask the frontend for a capability it declared in `hello` (`Capability::Browser`,
`Focus`, `Secrets`): `answer` gives the `HostReply` as each arrives, or the frontend replies later
with `CoreHandle::reply(id, reply)`.

## Game sessions

`kchess_games::computer::ComputerGame`, `local::LocalGame`, `puzzle::OnBoard` and
`archive::GameArchive` run the domain's typed state machines: the frontend gives them what they
ask of it when they ask (the clock, whether engine help is allowed) and carries out the effects
they return (searches, sounds, saving the session and its Game history entry, `history`). The app
and the CLI share them, so a game started in one carries on in the other.
