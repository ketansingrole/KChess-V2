# Mac App Store

KChess is on the Mac App Store as `com.kchess.app` (team `4NBN7U2PZL`), beside the GitHub
downloads. It is the same app run in the App Sandbox, built by `cargo xtask package --mas`
(`tooling/xtask/src/package/mas.rs`).

## How the store build differs

- **Sandboxed.** The app may use the network (client, and the loopback listener for the Lichess
  sign-in), the microphone, and files a person picks. The core service, Stockfish and the voice
  helper inherit its sandbox. Its profile lives in the container
  (`~/Library/Containers/com.kchess.app/Data/Library/Application Support/KChess`), apart from a
  GitHub install's.
- **Updated by the App Store.** `kchess_host::app_store()` (the sandbox's
  `APP_SANDBOX_CONTAINER_ID`) turns the updater off; Settings says the App Store updates it.
- **Bundled Stockfish only.** The store forbids downloading programs, and the sandbox cannot keep
  a chosen executable across launches, so the engine settings offer the bundled Stockfish alone.
- **No command line.** `kchess-cli` runs outside the sandbox and cannot reach the sandboxed core.
- **Core socket** in the container's own temporary folder (`kchess-client` `discovery.rs`).

Try the sandbox locally: `cargo xtask package --mas` without the signing variables signs the app
ad hoc in the sandbox and leaves it in `apps/gpui/dist/mas/KChess.app`.

## Release flow

1. **Tag push:** the Release workflow's `app-store` job (`app-store-build.yml`) builds the signed
   `.pkg` and uploads it to App Store Connect, version and build number both the release version.
   It waits in TestFlight. Manual runs only make the package as an artifact.
2. **Publishing the GitHub release:** `app-store-publish.yml` first sends the build to the
   **Public Beta** TestFlight group, whose public link (https://testflight.apple.com/join/qt2NQzhx)
   is on the site, with the release notes as What to Test, and submits it for beta review. That
   happens whatever App Review is doing. It then stages the App Store version with the
   listing in `app-store/` (and the release notes as What's New), attaches that build and submits
   it for review. Apple releases it when the review passes.
   While another version is with Apple (waiting for or in review, or approved and not yet
   released) it submits nothing, so that review never restarts: once that version is out, run
   Publish to the App Store by hand with the newest tag.

## What App Review reads

App Review asks how the app uses each sandbox entitlement (guideline 2.4.5), and holds the review
until it hears. Two places answer it, and both belong to each version:

- **App Review Information** notes: `app-store/review-information.json`. Publish to the App Store
  writes them (`asc review details set`) after staging the version. Only the file's fields are
  written; the contact (name, phone, email) stays as App Store Connect has it, so it never goes in
  this public repository.
- **App Sandbox Information**: one row per entitlement, in `app-store/sandbox-information.json`.
  App Store Connect takes these only through an Apple Account sign-in, which CI doesn't have, so
  the run's notice names the version to check. Before Apple reviews it, list the rows and set any
  that are missing or different:

  ```bash
  asc web review sandbox-entitlements list --version-id VERSION_ID
  ```

  ```bash
  asc web review sandbox-entitlements set --version-id VERSION_ID --entitlement KEY --justification "TEXT" --confirm
  ```

An entitlement added to the store build (`app_entitlements` in `tooling/xtask/src/package/mas.rs`)
needs its row in `app-store/sandbox-information.json`: xtask's tests fail without one. Change the
reviewer's notes in `app-store/review-information.json` when a feature they describe changes.

A failed upload: rerun the tag's `app-store` job; a build number is accepted once, so a build that
reached App Store Connect needs no rerun. A failed submission: rerun Publish to the App Store, or
run it by hand with the tag.

## Secrets and variables

| Name | Kind | What |
| --- | --- | --- |
| `MAS_APP_P12` | secret | Apple Distribution identity, base64 `.p12` |
| `MAS_INSTALLER_P12` | secret | Mac Installer Distribution identity, base64 `.p12` |
| `MAS_P12_PASSWORD` | secret | Both `.p12` files' password |
| `MAS_PROVISION_PROFILE` | secret | "KChess Mac App Store" profile, base64 |
| `APPLE_TEAM_ID` | secret | Already set for notarization |
| `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_PRIVATE_KEY` | secrets | App Store Connect API key ("KChess CI", App Manager) |
| `ASC_APP_ID` | variable | The app's App Store Connect ID |

The certificates and profile expire on 4 November 2027. Renew them with `asc certificates create`
and `asc profiles create` (`MAC_APP_STORE`, bundle ID resource `7XGZAY64Z7`), then replace the
secrets.

## Licensing

KChess is GPL-3.0-or-later and bundles Stockfish (GPL-3.0). The listing and the app's notices
point at both sources. If a Stockfish author objects to App Store distribution, remove the app
from sale; the GitHub downloads are unaffected.
