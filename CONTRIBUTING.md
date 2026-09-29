# Contributing to KChess

Thanks for helping! Bug reports, fixes, and ideas are all welcome.

## Getting started

Requires Node.js 22.12+ and npm (see the [README](README.md#run) for details).

```bash
npm install
npm run dev        # Nuxt + Electron together, with hot reload
```

Before opening a pull request, run the same checks CI runs:

```bash
npm run format:check   # fix with: npm run format
npm run typecheck
npm test
```

## Guidelines

- Keep changes focused; one topic per pull request. Describe what changed and why, and
  how you checked it (screenshots help for UI changes).
- Match the surrounding code: naming, comment density, and idiom. Prettier settings are in
  `.prettierrc.json`.
- Anything that crosses the Electron IPC boundary must be validated in
  `src/shared/validate.ts`. The renderer is sandboxed and must never choose what the main
  process executes.
- New settings need three things: the `Settings` type, the `SETTINGS_KEYS` list in
  `src/main/store.ts`, and a new entry at the end of `MIGRATIONS` in
  `src/main/migrations.ts` (never edit an earlier migration).
- Be gentle with Lichess. Bulk downloads run one request at a time, and requests should be
  attributed for the data-usage counters (`withUsage` in `src/main/usage.ts`).

## Licensing of contributions

KChess is licensed under the [GPL-3.0-or-later](LICENSE). By submitting a contribution you
agree that it is licensed under the same terms, and that you have the right to submit it.

**Only contribute work you own or that is clearly open-licensed.** In particular, do not add
images, icons, sounds, or fonts taken from stock sites, design tools (such as Canva), or
other sources whose terms forbid redistribution or use in open-source projects. Add any new
third-party code or asset to [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) with its license.

## Reporting security problems

Please do not open a public issue for a vulnerability; see [SECURITY.md](SECURITY.md).
