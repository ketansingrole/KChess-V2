# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue or pull request.

Use GitHub's private reporting: on the repository page choose **Security → Report a
vulnerability**. Include what you found, how to reproduce it, and the version or commit.

You can expect an acknowledgement within a few days. Fixes are released as soon as they are
ready, and you will be credited if you wish.

## What is in scope

KChess is a desktop app that stores a Lichess login on the user's computer, so these matter most:

- Leaking or mishandling Lichess OAuth tokens (they are encrypted with the operating
  system's credential storage and never leave the main process).
- Ways for renderer content to run code or choose which executable the main process starts
  (the renderer is sandboxed; all IPC input is validated in `src/shared/validate.ts`).
- Flaws in the OAuth loopback callback or in verifying downloaded Stockfish builds.

Vulnerabilities in Electron, Chromium, or other dependencies should be reported to those
projects; let us know too if KChess needs to ship an update.

## Supported versions

Only the latest release and the `main` branch receive fixes.
