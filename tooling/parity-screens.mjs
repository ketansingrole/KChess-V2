#!/usr/bin/env node
// Screenshots of every page of the built Electron app, on a fresh profile and a fixed window size,
// as the reference the GPUI app (apps/gpui) is compared with. Run after `pnpm run build`:
//
//   node tooling/with-branded-electron.mjs node tooling/parity-screens.mjs <output-dir> [light|dark] [profile]
//
// Naming a profile opens that one (one the GPUI app played in, say) and leaves it in place.
// The wrapper supplies ELECTRON_EXEC_PATH, as `pnpm run test:e2e` does.
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { _electron } from '@playwright/test'

const root = fileURLToPath(new URL('..', import.meta.url))
const out = process.argv[2]
const scheme = process.argv[3] === 'dark' ? 'dark' : 'light'
if (!out) throw new Error('Name a folder for the screenshots.')
const executablePath = process.env.ELECTRON_EXEC_PATH
if (!executablePath) throw new Error('Run through tooling/with-branded-electron.mjs.')

const PAGES = [
  'Home',
  'Play with Computer',
  'Over the board',
  'Puzzles',
  'Practice',
  'Openings',
  'Analysis board',
  'Game history',
  'Studies',
  'Board editor',
  'Play on Lichess',
  'Tournaments',
  'Watch',
  'Lichess insights',
  'Following',
  'Players',
]

mkdirSync(out, { recursive: true })
const given = process.argv[4]
const profile = given ?? mkdtempSync(join(tmpdir(), 'kchess-parity-'))
const app = await _electron.launch({
  executablePath,
  args: ['apps/desktop'],
  cwd: root,
  env: {
    ...process.env,
    KCHESS_NUXT_URL: '',
    KCHESS_USER_DATA_DIR: profile,
    KCHESS_STORE_DEBUG: '1',
  },
})
try {
  const page = await app.firstWindow()
  await page.emulateMedia({ colorScheme: scheme })
  await app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0]?.setContentSize(1280, 820)
  })
  await page.waitForSelector('.main-area')
  const shot = async (name) => {
    await page.waitForTimeout(600)
    await page.screenshot({ path: join(out, `${name}.png`) })
  }
  for (const [index, label] of PAGES.entries()) {
    await page.locator('aside').getByText(label, { exact: true }).click()
    await shot(`${String(index).padStart(2, '0')}-${label.toLowerCase().replaceAll(' ', '-')}`)
  }
  await page.locator('aside').getByRole('button', { name: 'Settings' }).click()
  await shot('16-settings')
  console.log(`parity: ${PAGES.length + 1} screenshots in ${out}`)
} finally {
  await app.close()
  if (!given) rmSync(profile, { recursive: true, force: true })
}
