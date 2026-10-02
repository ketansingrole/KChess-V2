import {
  test as base,
  expect,
  _electron,
  type ElectronApplication,
  type Page,
} from '@playwright/test'
import { cp, mkdtemp, rm, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { migrate } from '../../src/main/migrations'
import { storeSample } from '../../src/main/puzzleQueries'

const test = base.extend<{ desktop: { app: ElectronApplication; page: Page; profile: string } }>({
  desktop: async ({ playwright: _playwright }, use, testInfo) => {
    const profile = await mkdtemp(join(tmpdir(), 'kchess-e2e-'))
    if (testInfo.title.includes('voice'))
      await cp(join(process.cwd(), '.data', 'voice', 'cache'), join(profile, 'voice'), {
        recursive: true,
      })
    const db = new DatabaseSync(join(profile, 'kchess.db'))
    migrate(db)
    storeSample(db, [
      {
        id: 'promo123',
        fen: '6k1/P6p/8/8/8/8/5PPP/6K1 b - - 0 1',
        moves: 'h7h6 a7a8q',
        rating: 1500,
        themes: 'promotion',
      },
    ])
    db.close()
    let app: ElectronApplication | undefined
    try {
      const executablePath = process.env.ELECTRON_EXEC_PATH
      if (!executablePath)
        throw new Error(
          'Use npm run test:e2e or npm run test:packaged to launch through the branded wrapper.',
        )
      app = await _electron.launch({
        executablePath,
        args: [
          ...(process.env.KCHESS_PACKAGED_EXEC_PATH ? [] : ['.']),
          ...(testInfo.title.includes('voice')
            ? ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream']
            : []),
        ],
        env: { ...process.env, KCHESS_NUXT_URL: '', KCHESS_USER_DATA_DIR: profile },
      })
      await app.context().tracing.start({ screenshots: true, snapshots: true })
      const page = await app.firstWindow()
      await page.waitForSelector('.main-area')
      await expect(page.locator('aside').getByText('Dashboard', { exact: true })).toBeAttached()
      await use({ app, page, profile })
    } finally {
      if (app) {
        if (testInfo.status !== testInfo.expectedStatus) {
          try {
            await app
              .firstWindow()
              .then((page) => page.screenshot({ path: testInfo.outputPath('desktop.png') }))
          } catch {
            /* Window may have crashed. */
          }
        }
        await app
          .context()
          .tracing.stop(
            testInfo.status !== testInfo.expectedStatus
              ? { path: testInfo.outputPath('trace.zip') }
              : {},
          )
        await app.close()
      }
      await rm(profile, { recursive: true, force: true })
    }
  },
})

test('loads cached voice recognition offline and releases the microphone after a training run @packaged', async ({
  desktop: { app, page },
}) => {
  await app.evaluate(() => {
    globalThis.fetch = () =>
      Promise.reject(new Error('Offline voice test: network is unavailable.'))
  })
  await navigate(page, 'Practice')
  await page.getByRole('tab', { name: 'Say the square' }).click()
  await page.getByRole('switch', { name: 'Voice input', exact: true }).click()
  await expect(page.getByText('Ready for your next turn or run', { exact: true })).toBeVisible({
    timeout: 60_000,
  })
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  await expect(page.getByText('Listening…', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'End run', exact: true }).click()
  await expect(page.getByText('Ready for your next turn or run', { exact: true })).toBeVisible()
  await navigate(page, 'Play with Computer')
  // The game page shows the compact voice bar: a toggle button rather than a switch.
  const voice = page.getByRole('button', { name: 'Voice input', exact: true })
  const bar = page.locator('.voice-bar')
  await voice.click()
  await expect(bar).toHaveAttribute('data-state', 'listening', { timeout: 60_000 })
  await voice.click()
  await expect(bar).toHaveAttribute('data-state', 'off')
})

async function navigate(page: Page, label: string) {
  const link = page.locator('aside').getByText(label, { exact: true })
  if (await link.isVisible()) return link.click()
  // Below 1024px (e.g. the 1024×768 Windows CI screen) the sidebar is a slide-over menu instead.
  const menu = page.getByRole('dialog')
  if (!(await menu.isVisible())) await page.getByRole('button', { name: 'Show sidebar' }).click()
  await menu.getByText(label, { exact: true }).click()
  await expect(menu).toBeHidden()
}

/** Click board squares through Chessground's real pointer handlers. */
async function move(page: Page, from: string, to: string) {
  const board = page.locator('cg-board')
  const bounds = await board.boundingBox()
  if (!bounds) throw new Error('Board is not visible')
  for (const square of [from, to]) {
    const file = square.charCodeAt(0) - 97
    const rank = Number(square[1]) - 1
    await page.mouse.click(
      bounds.x + ((file + 0.5) * bounds.width) / 8,
      bounds.y + ((7 - rank + 0.5) * bounds.height) / 8,
    )
  }
}

test('launches with SQLite, sandboxed preload and bundled Stockfish @packaged', async ({
  desktop: { app, page, profile },
}) => {
  const runtime = await app.evaluate(({ app }) => ({
    version: process.versions.electron,
    packaged: app.isPackaged,
    profile: app.getPath('userData'),
  }))
  const expectedVersion = JSON.parse(
    await readFile(new URL('../../node_modules/electron/package.json', import.meta.url), 'utf8'),
  ).version
  expect(runtime.version).toBe(expectedVersion)
  expect(runtime.profile).toBe(profile)
  if (process.env.KCHESS_PACKAGED_EXEC_PATH) expect(runtime.packaged).toBe(true)
  const status = await page.evaluate(async () => {
    const data = await window.kchess.loadData()
    const engine = await window.kchess.engineStatus()
    const move = await window.kchess.bestMove([], 'low', { movetime: 100 })
    return { accounts: data.accounts, engine, move, nodeExposed: 'require' in window }
  })
  expect(status.accounts).toEqual([])
  expect(status.engine.ready).toBe(true)
  expect(status.engine.bundled).toBe(true)
  expect(status.move).toMatch(/^[a-h][1-8][a-h][1-8][qrbn]?$/)
  expect(status.nodeExposed).toBe(false)
})

test('plays a move through the board and keeps the game when navigating', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Play with Computer')
  await expect(page.getByText('Your move', { exact: true })).toBeVisible()
  await move(page, 'e2', 'e4')
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e4')
  await expect(page.getByText('Your move', { exact: true })).toBeVisible()
  await navigate(page, 'History')
  await navigate(page, 'Play with Computer')
  await expect(page.getByRole('list', { name: 'Moves', exact: true })).toContainText('e4')
})

test('keeps a failed offline puzzle verdict on remount and opens the promotion picker', async ({
  desktop: { page },
}) => {
  await navigate(page, 'Puzzles')
  await page.getByRole('tab', { name: 'Offline', exact: true }).click()
  await expect(page.locator('cg-board')).toBeVisible()
  await move(page, 'f2', 'f3')
  await expect(page.getByText('That’s not the move — try again', { exact: true })).toBeVisible()
  await navigate(page, 'History')
  await navigate(page, 'Puzzles')
  await expect(page.locator('cg-board')).toBeVisible()
  await move(page, 'a7', 'a8')
  const picker = page.getByRole('dialog', { name: 'Choose promotion piece' })
  await expect(picker).toBeVisible()
  await picker.getByRole('button', { name: 'Queen', exact: true }).click()
  await expect(
    page.getByText('Solved after a mistake — not counted', { exact: true }),
  ).toBeVisible()
})

test('persists appearance settings after reloading', async ({ desktop: { page } }) => {
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await page.getByRole('tab', { name: 'Dark', exact: true }).click()
  await expect
    .poll(async () =>
      page.evaluate(async () => (await window.kchess.loadData()).settings.appearance),
    )
    .toBe('dark')
  await page.reload()
  await expect(page.locator('html')).toHaveClass(/dark/)
  expect(
    await page.evaluate(async () => (await window.kchess.loadData()).settings.appearance),
  ).toBe('dark')
})

test('shows update capabilities and persists update preferences @packaged', async ({
  desktop: { app, page },
}) => {
  const status = await page.evaluate(() => window.kchess.appUpdateStatus())
  expect(status.currentVersion).toBe(await app.evaluate(({ app }) => app.getVersion()))
  if (status.canCheck) expect(status.phase).not.toBe('disabled')
  else expect(status.phase).toBe('disabled')
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await navigate(page, 'Updates')
  await expect(page.getByRole('heading', { name: 'Updates', level: 1 })).toBeVisible()
  await expect(
    page.getByText(`Installed version ${status.currentVersion} · Stable releases`),
  ).toBeVisible()
  const check = page.getByRole('button', { name: 'Check for updates', exact: true })
  const download = page.getByRole('switch', {
    name: 'Download updates in the background',
    exact: true,
  })
  if (status.canCheck) await expect(check).toBeEnabled()
  else await expect(check).toBeDisabled()
  if (status.canInstall) await expect(download).toBeEnabled()
  else await expect(download).toBeDisabled()
  if (status.canCheck) {
    await page.getByRole('switch', { name: 'Automatically check for updates', exact: true }).click()
    await expect
      .poll(() =>
        page.evaluate(async () => (await window.kchess.loadData()).settings.updateAutoCheck),
      )
      .toBe(false)
  }
  // Exercise the actual preload, IPC validation and SQLite round trip without downloading a release.
  await page.evaluate(async () => {
    const { settings } = await window.kchess.loadData()
    await window.kchess.saveSettings({
      ...settings,
      updateAutoCheck: false,
      updateAutoDownload: false,
      updateInstallOnQuit: false,
    })
  })
  await page.reload()
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await navigate(page, 'Updates')
  await expect(
    page.getByRole('switch', { name: 'Automatically check for updates', exact: true }),
  ).not.toBeChecked()
  await expect(download).not.toBeChecked()
  await expect(
    page.getByRole('switch', { name: 'Install updates when I quit', exact: true }),
  ).not.toBeChecked()
  await expect(page.evaluate(() => window.kchess.installAppUpdate())).rejects.toThrow(
    'Download an update',
  )
  await page.screenshot({ path: 'test-results/app-updates.png' })
})

test('exports redacted diagnostics through Settings', async ({
  desktop: { app, page, profile },
}) => {
  const destination = join(profile, 'diagnostics.json')
  await app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath })
    console.error('Diagnostic test: Bearer test-oauth-secret')
  }, destination)
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+,' : 'Control+,')
  await expect(
    page.getByRole('heading', { name: 'Appearance', exact: true, level: 1 }),
  ).toBeVisible()
  await navigate(page, 'Data & storage')
  await page.getByRole('button', { name: 'Export diagnostics', exact: true }).click()
  await expect(page.getByText('Diagnostics exported.', { exact: true })).toBeVisible()
  const report = await readFile(destination, 'utf8')
  expect(report).toContain('Bearer [redacted]')
  expect(report).not.toContain('test-oauth-secret')
  expect(Object.keys(JSON.parse(report))).toEqual([
    'generatedAt',
    'version',
    'platform',
    'arch',
    'versions',
    'logs',
  ])
})
